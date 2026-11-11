const directions = {
  right: ["KeyD", "ArrowRight"],
  left: ["KeyA", "ArrowLeft"],
  up: ["KeyW", "ArrowUp"],
  down: ["KeyS", "ArrowDown"],
};

export const defaults = {
  classic: {
    ...directions, fire: ["Space", "KeyZ"], sprint: ["ShiftLeft", "ShiftRight", "KeyX"],
    select: ["Tab"], start: ["Enter"], shoulderR: ["KeyE"], shoulderL: ["KeyQ"],
  },
  foot: {
    ...directions, fire: ["KeyJ", "MouseLeft"], sprint: ["ShiftLeft", "ShiftRight"],
    jump: ["Space"], vehicle: ["KeyF"], select: ["Tab"], start: ["Enter"],
    shoulderR: ["KeyH"], aim: ["MouseRight"],
  },
  vehicle: {
    ...directions, brake: ["Space"], horn: ["ShiftLeft", "ShiftRight"],
    mission: ["CapsLock"], turret: ["ControlLeft", "ControlRight"], vehicle: ["KeyF"],
    start: ["Enter"], select: ["Tab"],
  },
  shortcuts: { camera: ["KeyV"] },
};

export function validBinding(code) {
  return typeof code === "string" && /^[A-Za-z][A-Za-z0-9]{0,31}$/.test(code)
    && !["Escape", "Unidentified", "AltLeft", "AltRight", "MetaLeft", "MetaRight"].includes(code)
    && !/^F\d+$/.test(code);
}

export function restoreBindings(saved) {
  const result = structuredClone(defaults);
  for (const [profile, actions] of Object.entries(result)) {
    const used = new Set();
    for (const action of Object.keys(actions)) {
      const codes = saved?.[profile]?.[action];
      if (Array.isArray(codes) && (codes.length || profile === "shortcuts") && codes.length <= 4
        && codes.every(validBinding) && new Set(codes).size === codes.length) actions[action] = [...codes];
      if (actions[action].some(code => used.has(code))) {
        result[profile] = structuredClone(defaults[profile]);
        break;
      }
      for (const code of actions[action]) used.add(code);
    }
  }
  if (Object.entries(result).some(([profile, actions]) => profile !== "shortcuts" &&
    Object.values(actions).some(codes => codes.some(code => result.shortcuts.camera.includes(code))))) {
    result.shortcuts.camera = [];
  }
  return result;
}

const masks = {
  classic: { right: 16, left: 32, up: 64, down: 128, fire: 1, sprint: 2,
    select: 4, start: 8, shoulderR: 256, shoulderL: 512 },
  foot: { right: 16, left: 32, up: 64, down: 128, fire: 1, sprint: 2,
    jump: 3, vehicle: 512, select: 4, start: 8, shoulderR: 256 },
  vehicle: { right: 16, left: 32, up: 1, down: 2, brake: 256, horn: 320,
    mission: 768, vehicle: 512, start: 8, select: 4 },
};

export function keyboardMask(profile, held) {
  let bits = 0;
  for (const [action, mask] of Object.entries(masks[profile])) {
    if (profile === "foot" && action === "fire" && held("sprint")) continue;
    if (held(action)) bits |= mask;
  }
  if (profile === "vehicle" && held("turret") && (held("left") || held("right"))) bits |= 3;
  return 0x3ff ^ bits;
}

export function movement(bindings, profile, code) {
  if (profile !== "foot" && profile !== "vehicle") return undefined;
  return Object.keys(directions).find(direction => bindings[profile][direction].includes(code));
}

export function gamepadMask(pad) {
  if (!pad?.connected || pad.mapping !== "standard") return 0x3ff;
  const buttons = [0, 1, 8, 9, 15, 14, 12, 13, 5, 4];
  let mask = 0x3ff;
  for (const [bit, button] of buttons.entries()) {
    if (pad.buttons[button]?.pressed) mask &= ~(1 << bit);
  }
  const [x, y] = pad.axes;
  if (x > .25) mask &= ~16;
  if (x < -.25) mask &= ~32;
  if (y < -.25) mask &= ~64;
  if (y > .25) mask &= ~128;
  return mask;
}

export function mergeInputs(keyboard, gamepad) {
  let mask = keyboard & gamepad;
  for (const pair of [48, 192]) if ((mask & pair) === 0) mask |= pair;
  return mask;
}
