import { defaults, validBinding, restoreBindings, keyboardMask, movement, gamepadMask, mergeInputs } from "./input.mjs";

const launch = document.querySelector("#launch");
const start = document.querySelector("#start");
const status = document.querySelector("#status");
const canvas = document.querySelector("#screen");
const controls = document.querySelector("#controls");
const volume = document.querySelector("#volume");
const language = document.querySelector("#language");
const inputMode = document.querySelector("#input-mode");
const enhancedAudio = document.querySelector("#enhanced-audio");
const graphicsMode = document.querySelector("#graphics");
const textures = document.querySelector("#textures");
const hud = document.querySelector("#hud");
const saveKey = "advance-liberty:usa:save:v1";
const volumeKey = "advance-liberty:volume:v1";
const languageKey = "advance-liberty:language:v1";
const audioKey = "advance-liberty:enhanced-audio:v1";
const inputKey = "advance-liberty:input-mode:v1";
const frameMs = 1000 * 280896 / 16777216;
const bindingsKey = "advance-liberty:bindings:v1";
let bindings = restoreBindings();

try {
  bindings = restoreBindings(JSON.parse(localStorage.getItem(bindingsKey)));
} catch {
  // An invalid keymap is recoverable using the built-in defaults.
}
try {
  const saved = localStorage.getItem(volumeKey);
  if (saved !== null) volume.value = saved;
  enhancedAudio.checked = localStorage.getItem(audioKey) !== "false";
  inputMode.value = localStorage.getItem(inputKey) === "classic" ? "classic" : "enhanced";
  for (const setting of [graphicsMode, textures, hud]) {
    const saved = localStorage.getItem("advance-liberty:" + setting.id);
    if ([...setting.options].some(option => option.value === saved)) setting.value = saved;
  }
} catch {
  // Opening the save reports storage errors at launch.
}

function upload(core, bytes) {
  const address = core.upload(bytes.length);
  if (!address) throw new Error("Unexpected ROM or save size.");
  new Uint8Array(core.memory.buffer, address, bytes.length).set(bytes);
}

function savePreference(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch (error) {
    status.textContent = "Could not save preference. " + error.message;
  }
}

function setupLanguages(core, files) {
  const packs = new Map(files.map(file => [file.name, file]));
  language.replaceChildren(new Option("Original game", ""));
  for (const name of packs.keys()) language.add(new Option(name, name));
  let active = "";
  const change = async name => {
    language.disabled = true;
    try {
      const file = packs.get(name);
      if (file) {
        const address = core.upload_language(file.size);
        if (!address) throw new Error("Translation must be a non-empty JSON file up to 4 MiB.");
        const bytes = new Uint8Array(await file.arrayBuffer());
        new Uint8Array(core.memory.buffer, address, bytes.length).set(bytes);
      }
      if (!core.set_language(!file)) {
        const message = new Uint8Array(core.memory.buffer, core.error_ptr(), core.error_len());
        throw new Error(new TextDecoder().decode(message) || "Unable to apply translation.");
      }
      active = name;
      status.textContent = name ? "Translation applied." : "Original game language restored.";
      savePreference(languageKey, name);
    } catch (error) {
      status.textContent = error.message;
    } finally {
      language.value = active;
      language.disabled = false;
    }
  };
  language.onchange = () => void change(language.value);
  const preferred = localStorage.getItem(languageKey);
  const initial = preferred === "" || packs.has(preferred) ? preferred : packs.keys().next().value;
  return initial ? change(initial) : Promise.resolve();
}

function openSave(core) {
  let stored = localStorage.getItem(saveKey);
  if (stored !== null) {
    if (stored.length > 10924) throw new Error("The browser save has an invalid size.");
    upload(core, Uint8Array.from(atob(stored), char => char.charCodeAt(0)));
    if (!core.load_save()) throw new Error("The browser save could not be loaded.");
  }
  const read = () => new Uint8Array(core.memory.buffer, core.save(), core.save_len());
  let pending = read().slice();
  let dirty = false;
  let stable = 0;
  return () => {
    const bytes = read();
    if (bytes.some((value, index) => value !== pending[index])) {
      pending.set(bytes);
      dirty = true;
      stable = 0;
    }
    // Allow the game to finish writing all EEPROM blocks before committing.
    if (!dirty || ++stable < 60) return;
    if (localStorage.getItem(saveKey) !== stored) {
      throw new Error("Another tab updated this save. Reload to use its progress.");
    }
    const encoded = btoa(String.fromCharCode(...pending));
    localStorage.setItem(saveKey, encoded);
    stored = encoded;
    dirty = false;
    status.textContent = "Progress saved in this browser.";
  };
}

launch.addEventListener("submit", async event => {
  event.preventDefault();
  start.disabled = true;
  let context;
  try {
    const file = document.querySelector("#rom").files[0];
    if (!file || file.size !== 16 * 1024 * 1024) {
      throw new Error("Choose an unmodified USA ROM (16 MiB).");
    }
    status.textContent = "Loading game…";
    context = new AudioContext();
    await context.resume();
    const response = await fetch("advance_liberty_web.wasm");
    if (!response.ok) throw new Error("WebAssembly build not found.");
    const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {
      host: { now_ms: () => performance.now() },
    });
    const core = instance.exports;
    upload(core, new Uint8Array(await file.arrayBuffer()));
    if (!core.start()) throw new Error("This file does not match the supported USA ROM.");
    const updateSave = openSave(core);
    core.configure_audio(enhancedAudio.checked);
    core.configure_drops(30);
    const configure = () => core.configure_graphics(
      graphicsMode.value !== "original", graphicsMode.value === "wide",
      textures.value === "perspective", Number(hud.value),
    );
    configure();
    for (const setting of [graphicsMode, textures, hud]) {
      setting.onchange = () => {
        configure();
        savePreference("advance-liberty:" + setting.id, setting.value);
      };
    }
    savePreference(audioKey, String(enhancedAudio.checked));
    status.textContent = "";
    await setupLanguages(core, [...document.querySelector("#packs").files]);
    run(core, context, updateSave);
    launch.hidden = true;
    canvas.hidden = false;
    document.querySelector("#toolbar").hidden = false;
    canvas.focus();
  } catch (error) {
    if (context) void context.close();
    status.textContent = error.message;
    start.disabled = false;
  }
});

function run(core, context, updateSave) {
  const pressed = new Set();
  const suppressed = new Set();
  let previousHeld = new Set();
  let previousContext;
  const playing = new Set();
  const bindingProfile = document.querySelector("#binding-profile");
  const bindingList = document.querySelector("#bindings");
  const bindingStatus = document.querySelector("#binding-status");
  const controller = document.querySelector("#controller");
  let editing;
  let cursor = null;
  document.querySelector("#fullscreen").onclick = () => {
    const fullscreen = document.fullscreenElement ? document.exitFullscreen()
      : document.querySelector("#game").requestFullscreen();
    fullscreen.catch(error => { status.textContent = error.message; });
  };
  document.querySelector("#pause").onclick = pause;
  document.querySelector("#resume").onclick = () => controls.close();
  controls.addEventListener("close", () => {
    cancelBinding();
    resetTiming();
    canvas.focus();
  });
  controls.addEventListener("cancel", event => {
    if (editing) {
      event.preventDefault();
      cancelBinding();
    }
  });
  function pause() {
    if (controls.open) return;
    resetTiming();
    controls.showModal();
  }
  const labels = {
    fire: "Fire / A", sprint: "Sprint / B", select: "Select / change weapon", start: "In-game menu",
    shoulderR: "R", shoulderL: "L", vehicle: "Enter / exit vehicle",
    brake: "Parking brake", horn: "Horn", mission: "Special mission",
    turret: "Turret modifier", camera: "Cycle camera",
  };
  function drawBindings() {
    const profile = bindingProfile.value;
    bindingList.replaceChildren(...Object.entries(bindings[profile]).map(([action, codes]) => {
      const button = document.createElement("button");
      button.type = "button";
      const label = (profile === "vehicle" && { up: "Accelerate", down: "Brake / reverse" }[action])
        || labels[action] || action[0].toUpperCase() + action.slice(1);
      button.textContent = label + " — " + (codes.join(", ") || "Unassigned");
      button.onclick = () => {
        editing = { profile, action, button };
        bindingStatus.textContent = "Press a key or click this binding again with the desired mouse button. Escape cancels.";
      };
      button.onpointerdown = event => {
        if (editing?.button !== button) return;
        event.preventDefault();
        const code = ["MouseLeft", "MouseMiddle", "MouseRight"][event.button];
        if (code) assignBinding(code);
      };
      button.oncontextmenu = event => event.preventDefault();
      return button;
    }));
  }
  function cancelBinding() {
    editing = undefined;
    bindingStatus.textContent = "";
  }
  function assignBinding(code) {
    const { profile, action } = editing;
    const conflicts = profile === "shortcuts"
      ? Object.entries(bindings).filter(([name]) => name !== "shortcuts").flatMap(([, actions]) => Object.values(actions))
      : [bindings.shortcuts.camera, ...Object.entries(bindings[profile])
        .filter(([name]) => name !== action).map(([, codes]) => codes)];
    if (!validBinding(code) || conflicts.some(codes => codes.includes(code))) {
      bindingStatus.textContent = "That input is reserved or already assigned. Choose another.";
      return;
    }
    bindings[profile][action] = [code];
    savePreference(bindingsKey, JSON.stringify(bindings));
    cancelBinding();
    drawBindings();
  }
  bindingProfile.onchange = () => { cancelBinding(); drawBindings(); };
  document.querySelector("#reset-bindings").onclick = () => {
    const profile = bindingProfile.value;
    bindings[profile] = structuredClone(defaults[profile]);
    if (Object.entries(bindings).some(([name, actions]) => name !== "shortcuts" &&
      Object.values(actions).some(codes => codes.some(code => bindings.shortcuts.camera.includes(code))))) {
      bindings.shortcuts.camera = [];
    }
    savePreference(bindingsKey, JSON.stringify(bindings));
    cancelBinding();
    drawBindings();
  };
  drawBindings();
  const graphics = canvas.getContext("2d");
  let image;
  const gain = context.createGain();
  gain.gain.value = Number(volume.value) / 100;
  gain.connect(context.destination);
  inputMode.addEventListener("change", () => {
    pressed.clear();
    savePreference(inputKey, inputMode.value);
  });
  volume.addEventListener("input", () => {
    gain.gain.value = Number(volume.value) / 100;
    savePreference(volumeKey, volume.value);
  });
  window.addEventListener("keydown", event => {
    if (editing) {
      event.preventDefault();
      if (event.repeat) return;
      if (event.code === "Escape") cancelBinding();
      else if (!event.altKey && !event.metaKey) assignBinding(event.code);
      return;
    }
    if (controls.open) return;
    if (event.code === "Escape" && !event.repeat) {
      event.preventDefault();
      pause();
      return;
    }
    if (event.target instanceof HTMLInputElement || event.target instanceof HTMLSelectElement) return;
    if (event.metaKey || event.altKey) return;
    if (!Object.values(bindings).some(actions => Object.values(actions).some(codes => codes.includes(event.code)))) return;
    if (event.ctrlKey && !bindings.vehicle.turret.includes(event.code) &&
      !(inputMode.value === "enhanced" && core.control_context() === 2 &&
        (bindings.vehicle.left.includes(event.code) || bindings.vehicle.right.includes(event.code)))) return;
    event.preventDefault();
    pressed.add(event.code);
  });
  window.addEventListener("keyup", event => pressed.delete(event.code));
  const mouseButtons = ["MouseLeft", "MouseMiddle", "MouseRight"];
  // Mouse events report each button in a chord. Pointer down/up report only
  // the first press and last release, hiding fire while aim is already held.
  canvas.addEventListener("mousedown", event => {
    const code = mouseButtons[event.button];
    if (!code) return;
    event.preventDefault();
    canvas.focus();
    updateCursor(event);
    pressed.add(code);
  });
  canvas.addEventListener("contextmenu", event => event.preventDefault());
  function updateCursor(event) {
    const rect = canvas.getBoundingClientRect();
    const scale = Math.min(rect.width / canvas.width, rect.height / canvas.height);
    const width = canvas.width * scale, height = canvas.height * scale;
    cursor = [(event.clientX - rect.left - (rect.width - width) / 2) / width,
      (event.clientY - rect.top - (rect.height - height) / 2) / height];
  }
  window.addEventListener("mousemove", updateCursor);
  window.addEventListener("mouseup", event => {
    const code = mouseButtons[event.button];
    if (code) {
      pressed.delete(code);
      suppressed.delete(code);
    }
  });
  function releaseInput() {
    pressed.clear();
    suppressed.clear();
    previousHeld.clear();
  }
  window.addEventListener("pointercancel", releaseInput);
  window.addEventListener("blur", releaseInput);
  let nextAudio = 0;
  let lastTime;
  let elapsed = 0;
  function resetTiming() {
    releaseInput();
    lastTime = undefined;
    elapsed = 0;
    nextAudio = 0;
    for (const source of playing) source.stop();
    playing.clear();
  }
  document.addEventListener("visibilitychange", resetTiming);

  function queueAudio() {
    const length = core.audio_len();
    if (!length || context.state !== "running") return;
    const now = context.currentTime;
    if (nextAudio < now) nextAudio = now + .04;
    if (nextAudio > now + .12) return;
    const samples = new Int16Array(core.memory.buffer, core.audio(), length);
    const buffer = context.createBuffer(2, length / 2, 65536);
    for (let channel = 0; channel < 2; channel++) {
      const output = buffer.getChannelData(channel);
      for (let i = 0; i < output.length; i++) output[i] = samples[i * 2 + channel] / 32768;
    }
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.connect(gain);
    playing.add(source);
    source.onended = () => playing.delete(source);
    source.start(nextAudio);
    nextAudio += buffer.duration;
  }

  function draw() {
    const width = core.video_width(), height = core.video_height();
    if (!image || image.width !== width || image.height !== height) {
      canvas.width = width;
      canvas.height = height;
      image = graphics.createImageData(width, height);
    }
    const pixels = new Uint16Array(core.memory.buffer, core.pixels(), width * height);
    for (let i = 0; i < pixels.length; i++) {
      for (let channel = 0; channel < 3; channel++) {
        const value = (pixels[i] >> (channel * 5)) & 31;
        image.data[i * 4 + channel] = (value << 3) | (value >> 2);
      }
      image.data[i * 4 + 3] = 255;
    }
    graphics.putImageData(image, 0, 0);
  }

  let previousCamera = false;
  function gameInput(pad) {
    const enhanced = inputMode.value === "enhanced";
    const profile = enhanced ? ["classic", "foot", "vehicle"][core.control_context()] : "classic";
    for (const code of suppressed) if (!pressed.has(code) || !enhanced) suppressed.delete(code);
    if (enhanced && previousContext && profile !== previousContext) {
      for (const code of pressed) {
        if (!previousHeld.has(code)) continue;
        const direction = movement(bindings, profile, code);
        if (!direction || movement(bindings, previousContext, code) !== direction) suppressed.add(code);
      }
    }
    previousContext = enhanced ? profile : undefined;
    previousHeld = new Set(pressed);
    const held = action => bindings[profile][action].some(code => pressed.has(code) && !suppressed.has(code));
    const camera = bindings.shortcuts.camera.some(code => pressed.has(code));
    if (camera && !previousCamera && profile !== "classic") {
      const mode = core.cycle_camera();
      if (mode >= 0) status.textContent = "Camera: " + ["Auto", "Low", "Medium", "High"][mode];
    }
    previousCamera = camera;
    return {
      keys: mergeInputs(keyboardMask(profile, held), gamepadMask(pad)),
      aiming: profile === "foot" && held("aim"),
    };
  }

  let previousGuide = false;
  function readGamepad() {
    let pad;
    try {
      pad = Array.from(navigator.getGamepads?.() ?? []).find(p => p?.connected && p.mapping === "standard");
    } catch {
      // The browser may block the Gamepad API in embedded pages.
    }
    const description = pad ? "Gamepad connected" : "";
    if (controller.textContent !== description) controller.textContent = description;
    const guide = Boolean(pad?.buttons[16]?.pressed);
    if (guide && !previousGuide && !editing) {
      if (controls.open) controls.close();
      else pause();
    }
    previousGuide = guide;
    return pad;
  }

  function tick(now) {
    try {
      const pad = document.hidden ? undefined : readGamepad();
      if (!document.hidden && !controls.open) {
        elapsed += Math.min(lastTime === undefined ? frameMs : now - lastTime, frameMs * 2);
        lastTime = now;
        let drewFrame = false;
        while (elapsed >= frameMs) {
          const { keys, aiming } = gameInput(pad);
          if (!core.frame(keys, aiming && cursor !== null,
            cursor?.[0] ?? 0, cursor?.[1] ?? 0)) throw new Error("The game could not complete a frame.");
          updateSave();
          queueAudio();
          elapsed -= frameMs;
          drewFrame = true;
        }
        if (drewFrame) draw();
      }
      requestAnimationFrame(tick);
    } catch (error) {
      void context.close();
      status.textContent = error.message;
    }
  }
  requestAnimationFrame(tick);
}

