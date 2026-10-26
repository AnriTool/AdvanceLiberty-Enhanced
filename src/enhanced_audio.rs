//! Enhanced Output DSP adapted from JRickey/gba-recomp, revision
//! de4edf59b872d887046d6a3b005e2df551b6d44c, crates/recomp/src/main.rs.
//! MIT / Apache-2.0; licenses retained under vendor/.
//! Runs on the producer's exact 65536 Hz timeline, so no queue-fill pitch
//! correction is needed. Unity output gain keeps comparisons level matched.
use std::collections::VecDeque;

pub struct Output {
    fifo: [FifoInterp; 2],
    events: [VecDeque<(i8, u32)>; 2],
    psg: SincResampler,
    psg_samples: VecDeque<f32>,
    dc: [DcBlock; 2],
    samples: Vec<i16>,
}
impl Output {
    pub fn new() -> Self {
        Self {
            fifo: std::array::from_fn(|_| FifoInterp::new(65536.0)),
            events: Default::default(),
            psg: SincResampler::new(65536.0, 65536.0),
            psg_samples: VecDeque::new(),
            dc: std::array::from_fn(|_| DcBlock::new(65536.0)),
            samples: Vec::new(),
        }
    }
    pub fn frame(&mut self, bus: &mut gba_core::MemMap) -> &[i16] {
        for (queue, tap) in self.events.iter_mut().zip(bus.fifo_tap.iter_mut()) {
            queue.extend(tap.drain(..));
        }
        self.psg_samples
            .extend(bus.psg_tap.drain(..).map(|s| s as f32 / 32768.0));
        let cnt = u16::from_le_bytes([bus.io[0x82], bus.io[0x83]]);
        self.samples.clear();
        for _ in 0..bus.audio_buf.len() / 2 {
            let (mut l, mut r) = self.psg.next(&mut self.psg_samples);
            for f in 0..2 {
                let value = self.fifo[f].next(&mut self.events[f])
                    * if cnt & (4 << f) == 0 { 0.5 } else { 1.0 };
                let route = (cnt >> (8 + 4 * f)) & 3;
                if route & 2 != 0 {
                    l += value;
                }
                if route & 1 != 0 {
                    r += value;
                }
            }
            for (side, value) in [l, r].into_iter().enumerate() {
                let value = if bus.io[0x84] & 0x80 == 0 { 0.0 } else { value };
                let sample = self.dc[side].next(soft_clip(value));
                self.samples
                    .push((sample * 32768.0).round().clamp(-32768.0, 32767.0) as i16);
            }
        }
        // Bound unused FIFO history when a timer is reconfigured or disabled.
        // Keep the newest frame rather than letting an inactive tap grow.
        for queue in &mut self.events {
            while queue.len() > 4096 {
                queue.pop_front();
            }
        }
        &self.samples
    }
}

const SINC_TAPS: usize = 24;
const SINC_PHASES: usize = 128;
/// Hardware clip rail in the float mix domain (±0x200 units = ±0.5).
const RAIL: f32 = 0.5;
/// Unity gain retains our existing hardware-output loudness.
const OUT_GAIN: f32 = 1.0;

/// Stage-2 soft-knee saturator (enhanced path only): exact identity
/// through the hardware rail, then a tanh knee compressing the over-rail
/// region (the six-channel sum can reach 3x the rail) into the headroom
/// above it, asymptoting at full scale after OUT_GAIN. C1 at the knee.
/// Faithful mode keeps the hard rail — clipping is sometimes the
/// intended texture (the research note's "GBA speaker" mono examples).
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= RAIL {
        return x;
    }
    let h = 1.0 / OUT_GAIN - RAIL; // headroom above the rail
    (RAIL + h * ((a - RAIL) / h).tanh()).copysign(x)
}

/// Polyphase interpolation table: (SINC_PHASES + 1) rows of SINC_TAPS
/// coefficients, Hann-windowed sinc at cutoff `fc` (cycles per source
/// sample), each row normalized to unity DC gain.
fn build_sinc_table(fc: f64) -> Vec<f32> {
    use std::f64::consts::PI;
    let center = (SINC_TAPS / 2 - 1) as f64;
    let half = SINC_TAPS as f64 / 2.0;
    let mut table = Vec::with_capacity((SINC_PHASES + 1) * SINC_TAPS);
    for p in 0..=SINC_PHASES {
        let phi = p as f64 / SINC_PHASES as f64;
        let mut row = [0.0f64; SINC_TAPS];
        let mut sum = 0.0;
        for (k, c) in row.iter_mut().enumerate() {
            let t = k as f64 - center - phi;
            let sinc = if t.abs() < 1e-9 {
                2.0 * fc
            } else {
                (2.0 * PI * fc * t).sin() / (PI * t)
            };
            let win = if t.abs() < half {
                0.5 * (1.0 + (PI * t / half).cos())
            } else {
                0.0
            };
            *c = sinc * win;
            sum += *c;
        }
        table.extend(row.iter().map(|c| (c / sum) as f32));
    }
    table
}

/// One-pole DC blocker, fc ~ 10 Hz: y[n] = x[n] - x[n-1] + r*y[n-1].
/// Kills bias offsets, held-DAC ledges, and underrun pops.
struct DcBlock {
    r: f32,
    x1: f32,
    y1: f32,
}

impl DcBlock {
    fn new(out_hz: f64) -> DcBlock {
        DcBlock {
            r: 1.0 - (2.0 * std::f64::consts::PI * 10.0 / out_hz) as f32,
            x1: 0.0,
            y1: 0.0,
        }
    }

    fn next(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// Fixed-ratio polyphase windowed-sinc resampler over an interleaved
/// stereo queue. Band-limits to the narrower of the two Nyquists, so
/// downsampling to a 44.1/48 kHz device does not fold images the way
/// the zero-order-hold path does. Used for the PSG grid stream.
struct SincResampler {
    table: Vec<f32>,
    /// Last SINC_TAPS source samples per side, oldest first.
    hist: [[f32; SINC_TAPS]; 2],
    ratio: f64,
    frac: f64,
}

impl SincResampler {
    fn new(src_hz: f64, out_hz: f64) -> SincResampler {
        // 10% under the narrower Nyquist leaves the window transition
        // band room.
        SincResampler {
            table: build_sinc_table(0.45 * (out_hz / src_hz).min(1.0)),
            hist: [[0.0; SINC_TAPS]; 2],
            ratio: src_hz / out_hz,
            frac: 0.0,
        }
    }

    fn next(&mut self, q: &mut std::collections::VecDeque<f32>) -> (f32, f32) {
        // Follow the producer clock exactly; an underrun repeats
        // the newest pair (a one-sample ZOH the DC blocker smooths).

        self.frac += self.ratio;
        while self.frac >= 1.0 {
            self.frac -= 1.0;
            let (l, r) = match (q.pop_front(), q.pop_front()) {
                (Some(l), Some(r)) => (l, r),
                _ => (self.hist[0][SINC_TAPS - 1], self.hist[1][SINC_TAPS - 1]),
            };
            for (h, s) in self.hist.iter_mut().zip([l, r]) {
                h.copy_within(1.., 0);
                h[SINC_TAPS - 1] = s;
            }
        }
        let row = &self.table[(self.frac * SINC_PHASES as f64) as usize * SINC_TAPS..][..SINC_TAPS];
        (
            row.iter()
                .zip(self.hist[0].iter())
                .map(|(c, s)| c * s)
                .sum(),
            row.iter()
                .zip(self.hist[1].iter())
                .map(|(c, s)| c * s)
                .sum(),
        )
    }
}

/// Renders one Direct Sound channel as band-limited steps (the blip-buf
/// idea): each DAC transition deposits a windowed-sinc impulse — the
/// step's derivative — into a small future ring at its exact fractional
/// output position, and a leaky integrator reconstructs the waveform.
/// Edges land jitter-free at the channel's own timer rate (which may
/// change at any event), and crucially the DAC staircase spectrum — the
/// ZOH images above the mix rate's Nyquist — survives up to the device
/// Nyquist. That brightness is part of the hardware's canon sound;
/// smooth sample interpolation removes it and audibly dulls transients
/// (reference-trace comparison on a square-timbre chime). Channels
/// driven above the device rate degrade gracefully: overlapping
/// deposits sum to a proper low-pass.
struct FifoInterp {
    /// Windowed-sinc impulse table (rows sum to 1 = unit step).
    table: Vec<f32>,
    /// Future deposits, ring-indexed by output sample.
    fut: [f32; SINC_TAPS * 2],
    pos: usize,
    /// Leaky integrator state. The tiny leak only bounds drift; real
    /// output coupling is the shared DC blocker downstream.
    y: f32,
    /// Current DAC level, for edge deltas.
    level: f32,
    /// Master cycles until the next DAC event is due.
    t_next: f64,
    /// Current source sample spacing in master cycles.
    period: f64,
    /// Master cycles per output sample.
    out_step: f64,
}

impl FifoInterp {
    fn new(out_hz: f64) -> FifoInterp {
        FifoInterp {
            table: build_sinc_table(0.45),
            fut: [0.0; SINC_TAPS * 2],
            pos: 0,
            y: 0.0,
            level: 0.0,
            t_next: 512.0,
            period: 512.0, // placeholder until the first DAC event
            out_step: (1u64 << 24) as f64 / out_hz,
        }
    }

    fn next(&mut self, q: &mut std::collections::VecDeque<(i8, u32)>) -> f32 {
        // Producer runs on emulated time, independent of device buffering.

        let stepc = self.out_step;

        self.t_next -= stepc;
        while self.t_next <= 0.0 {
            // The event fires inside this output sample, at fraction phi
            // of the sample period.
            let phi = (1.0 + self.t_next / stepc).clamp(0.0, 1.0);
            let new_level = match q.pop_front() {
                Some((s, p)) => {
                    self.period = (p as f64).max(1.0);
                    s as f32 * (64.0 / 32768.0)
                }
                None => self.level, // DAC holds on underrun
            };
            let delta = new_level - self.level;
            self.level = new_level;
            if delta != 0.0 {
                let row =
                    &self.table[(phi * SINC_PHASES as f64) as usize * SINC_TAPS..][..SINC_TAPS];
                for (k, c) in row.iter().enumerate() {
                    self.fut[(self.pos + k) % (SINC_TAPS * 2)] += delta * c;
                }
            }
            self.t_next += self.period;
        }

        self.y = self.y * 0.9999 + self.fut[self.pos];
        self.fut[self.pos] = 0.0;
        self.pos = (self.pos + 1) % (SINC_TAPS * 2);
        self.y
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn band_limit_preserves_low_tones_and_rejects_alias_band() {
        let amplitude = |hz: f64| {
            let mut resampler = SincResampler::new(65536.0, 48000.0);
            let mut input = VecDeque::new();
            for i in 0..8000 {
                let x = (i as f64 * hz * std::f64::consts::TAU / 65536.0).sin() as f32;
                input.extend([x, -x]);
            }
            let mut energy = 0.0;
            for i in 0..5000 {
                let (l, r) = resampler.next(&mut input);
                assert!((l + r).abs() < 1e-6);
                if i >= 100 {
                    energy += l as f64 * l as f64;
                }
            }
            (energy / 4900.0).sqrt()
        };
        let low = amplitude(1000.0);
        assert!((low - 0.707).abs() < 0.02);
        assert!(amplitude(30000.0) < low * 0.02);
    }
    #[test]
    fn output_honors_stereo_routing_and_preserves_canonical_samples() {
        let mut bus = gba_core::MemMap::new(vec![0; 4]);
        bus.io[0x84] = 0x80;
        bus.io[0x83] = 2; // FIFO A left only, half volume.
        bus.audio_buf = vec![123; 2048];
        bus.psg_tap = vec![0; 2048];
        bus.fifo_tap[0] = (0..512)
            .map(|i| (if i % 16 < 8 { 64 } else { -64 }, 512))
            .collect();
        let original = bus.audio_buf.clone();
        let mut output = Output::new();
        let samples = output.frame(&mut bus);
        assert_eq!(samples.len(), original.len());
        assert!(samples.chunks_exact(2).any(|p| p[0] != 0));
        assert!(samples.chunks_exact(2).all(|p| p[1] == 0));
        assert_eq!(bus.audio_buf, original);
        assert!(bus.fifo_tap[0].is_empty());
        assert!(bus.psg_tap.is_empty());
        let mut dc = DcBlock::new(65536.0);
        let mut last = 0.0;
        for _ in 0..65536 {
            last = dc.next(0.25);
        }
        assert!(last.abs() < 1e-6);
        assert_eq!(soft_clip(0.25), 0.25);
        assert!(soft_clip(2.0) > 0.5 && soft_clip(2.0) < 1.0);
    }
}
