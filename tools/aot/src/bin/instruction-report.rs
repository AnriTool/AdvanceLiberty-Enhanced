use std::{collections::BTreeMap, fs};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("Usage: instruction-report AUDIT.csv")?;
    let text = fs::read_to_string(path)?;
    let mut groups: BTreeMap<String, u64> = BTreeMap::new();
    let mut hot = Vec::new();
    for row in text.lines().skip(1) {
        let r: Vec<_> = row.split(',').collect();
        let key = u32::from_str_radix(r[1], 16)?;
        let bytes = r[5];
        if bytes.len() < 8 {
            continue;
        }
        let mut word = 0u32;
        for i in 0..4 {
            word |= (u8::from_str_radix(&bytes[i * 2..i * 2 + 2], 16)? as u32) << (8 * i);
        }
        let instr = if key & 1 != 0 {
            armv4t::decode_thumb(word as u16, key & !1)
        } else {
            armv4t::decode_arm(word, key)
        };
        let detail = format!("{:?}", instr.op);
        let group = detail.split(['{', '(']).next().unwrap().trim().to_owned();
        let count = r[2].parse::<u64>()?;
        *groups.entry(group).or_default() += count;
        hot.push((count, key, detail));
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    for (op, count) in groups {
        println!("{count:10} {op}");
    }
    hot.sort_by_key(|g| std::cmp::Reverse(g.0));
    for (count, key, detail) in hot.into_iter().take(30) {
        println!("{key:08x} {count:10} {detail}");
    }
    Ok(())
}
