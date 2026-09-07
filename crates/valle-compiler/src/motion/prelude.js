
// Typed values must not be coerced into strings or numbers.
//
// Apply typed-value admission to QuickJS folding as well as the expression IR.
//
// Throw a marked error through a nonenumerable Symbol.toPrimitive hook, preserving the JSON
// representation of typed values.
const __typed = (value) => {
  Object.defineProperty(value, Symbol.toPrimitive, {
    value: () => {
      throw new Error("valle:typed:a `" + value.__valleType +
        "` value cannot be coerced into a string or a number; keep it as a typed value " +
        "(a template literal hole accepts one directly)");
    },
    enumerable: false,
  });
  return value;
};
const defineControls = (value) => value;
const __control = (kind, options = {}) => ({ kind, ...options });
const number = (options = {}) => __control("number", options);
const string = (options = {}) => __control("string", options);
const boolean = (a = {}, b, op) => b === undefined
  ? __control("bool", a)
  : __typed({ __valleType: "pathBoolean", left: a, right: b, op });
const color = (options = {}) => __control("color", options);
const array = (items, options = {}) => __control("array", { items, ...options });
const record = (fields) => __control("record", { fields });
const tuple = (items) => __control("tuple", { items });
const length = (options = {}) => __control("length", options);
const angle = (options = {}) => __control("angle", options);
const point = (x = {}, y) => y === undefined && typeof x === "object"
  ? __control("point", x)
  : __typed({ __valleType: "point", x, y });
const rect = (x = {}, y, width, height) => y === undefined && typeof x === "object"
  ? __control("rect", x)
  : __typed({ __valleType: "rect", x, y, width, height });
const path = (value = {}) => typeof value === "object" && value.__valleType === undefined
  ? __control("pathData", value)
  : __typed({ __valleType: "pathData", d: value });
const line = (points) => __typed({ __valleType: "pathLine", points });
const cubic = (from, control1, control2, to) => __typed({
  __valleType: "pathCubic", from, control1, control2, to,
});
const arc = (center, radius, startAngle, endAngle) => __typed({
  __valleType: "pathArc", center, radius, startAngle, endAngle,
});
const area = (input, baseline) => __typed({ __valleType: "pathArea", input, baseline });
const offsetPath = (input, distance) => __typed({ __valleType: "pathOffset", input, distance });
const displacement = (seed, frequency, scale, options = {}) => __typed({
  __valleType: "nodeDisplacement", seed, frequency, scale, options,
});
const motionBlur = (velocity, shutterAngle = 180) => __typed({
  __valleType: "nodeMotionBlur", velocity, shutterAngle,
});
const motionPath = (input, progress, options = {}) => __typed({
  __valleType: "motionPath", input, progress, options,
});
// Provide the prepare-time equivalent of runtime clamp.
//
// Match lower_clamp's min(max(value, low), high) evaluation order.
const clamp = (value, low, high) => {
  // Reject reversed bounds before constant folding can bypass runtime admission.
  if (low > high) {
    throw new Error("valle:compute:clamp(value, low, high) needs low <= high, but low is "
      + low + " and high is " + high);
  }
  const lifted = value > low ? value : low;   // = fold_pairwise(Gt, [value, low])
  return lifted < high ? lifted : high;       // = fold_pairwise(Lt, [lifted, high])
};
// Freeze a component-local frame clock while the scene camera keeps sampling the real frame.
// The runtime lowering in `lower_freeze_frame` is deliberately isomorphic to this prepare-time
// implementation so a static call and a frame-varying call cannot disagree at the boundaries.
const freezeFrame = (frame, options) => {
  if (options === null || typeof options !== "object" || Array.isArray(options)) {
    throw new Error("valle:compute:freezeFrame(frame, { from, to }) expects an options object");
  }
  const keys = Object.keys(options);
  if (keys.some((key) => key !== "from" && key !== "to")) {
    throw new Error("valle:compute:freezeFrame accepts only from and to");
  }
  const { from, to } = options;
  if (!Number.isInteger(from) || !Number.isInteger(to) || from < 0 || to <= from) {
    throw new Error("valle:compute:freezeFrame needs integer frames with 0 <= from < to");
  }
  return frame < from ? frame : frame < to ? from : frame - (to - from);
};
const gradientStop = (offset, color) => __typed({ __valleType: "gradientStop", offset, color });
const fitText = (options = {}) => {
  if (options === null || typeof options !== "object" || Array.isArray(options)) {
    throw new Error("valle:compute:fitText expects { minFontSize, maxFontSize }");
  }
  const keys = Object.keys(options);
  if (keys.some((key) => key !== "minFontSize" && key !== "maxFontSize")) {
    throw new Error("valle:compute:fitText accepts only minFontSize and maxFontSize");
  }
  const minFontSize = options.minFontSize;
  const maxFontSize = options.maxFontSize;
  if (!Number.isFinite(minFontSize) || !Number.isFinite(maxFontSize) ||
      minFontSize <= 0 || maxFontSize < minFontSize) {
    throw new Error("valle:compute:fitText needs 0 < minFontSize <= maxFontSize");
  }
  return __typed({ __valleType: "fitText", minFontSize, maxFontSize });
};
const linearGradient = (start, end, stops, spread = "pad") => __typed({
  __valleType: "linearGradient", start, end, stops, spread,
});
const radialGradient = (center, radius, stops, spread = "pad") => __typed({
  __valleType: "radialGradient", center, radius, stops, spread,
});
const conicGradient = (center, startAngle, stops, spread = "pad") => __typed({
  __valleType: "conicGradient", center, startAngle, stops, spread,
});
const nodeTarget = (options = {}) => __control("nodeTarget", options);
const select = (options = {}) => __control("select", options);
// `frames` is intentionally overloaded: object input remains the existing controls constructor;
// a finite number is a Sequence time literal. Both are prepare-only values, so no runtime
// ambiguity reaches the Artifact.
const __sequenceTime = (unit, value) => ({ __valleType: "sequenceTime", unit, value });
const frames = (value = {}) => typeof value === "number"
  ? __sequenceTime("frames", value)
  : __control("frames", value);
const seconds = (value) => __sequenceTime("seconds", value);
const stage = (options = {}) => ({ __valleType: "sequenceStageInput", ...options });
const defineSequence = (input) => {
  if (input === null || typeof input !== "object" || Array.isArray(input)) {
    throw new Error("valle:compute:defineSequence expects an object of named stages");
  }
  const entries = Object.entries(input);
  if (entries.length === 0) {
    throw new Error("valle:compute:defineSequence needs at least one stage");
  }
  if (entries.length > 256) {
    throw new Error("valle:compute:defineSequence exceeds the 256-stage budget");
  }
  const out = {};
  let sequenceUnit = null;
  for (let index = 0; index < entries.length; index += 1) {
    const [label, spec] = entries[index];
    if (!spec || spec.__valleType !== "sequenceStageInput") {
      throw new Error("valle:compute:sequence stage `" + label + "` must use stage({...})");
    }
    const allowed = new Set(["__valleType", "at", "after", "delay", "overlap", "duration"]);
    for (const key of Object.keys(spec)) {
      if (!allowed.has(key)) {
        throw new Error("valle:compute:sequence stage `" + label + "` has unknown option `" + key + "`");
      }
    }
    const duration = spec.duration;
    if (!duration || duration.__valleType !== "sequenceTime" ||
        !Number.isFinite(duration.value) || duration.value <= 0) {
      throw new Error("valle:compute:sequence stage `" + label + "` needs a finite positive duration");
    }
    if (sequenceUnit === null) sequenceUnit = duration.unit;
    if (duration.unit !== sequenceUnit) {
      throw new Error("valle:compute:sequence cannot mix seconds() and frames() units");
    }
    if (spec.at !== undefined && spec.after !== undefined) {
      throw new Error("valle:compute:sequence stage `" + label + "` cannot use both at and after");
    }
    if (spec.delay !== undefined && spec.overlap !== undefined) {
      throw new Error("valle:compute:sequence stage `" + label + "` cannot use both delay and overlap");
    }
    if ((spec.delay !== undefined || spec.overlap !== undefined) && spec.after === undefined) {
      throw new Error("valle:compute:sequence stage `" + label + "` uses delay/overlap without after");
    }
    const timeValue = (name, value, fallback = 0) => {
      if (value === undefined) return fallback;
      if (!value || value.__valleType !== "sequenceTime" || value.unit !== sequenceUnit ||
          !Number.isFinite(value.value) || value.value < 0) {
        throw new Error("valle:compute:sequence stage `" + label + "` `" + name +
          "` must be a finite non-negative " + sequenceUnit + " value");
      }
      return value.value;
    };
    let start;
    if (spec.at !== undefined) {
      start = timeValue("at", spec.at);
    } else if (spec.after !== undefined) {
      if (typeof spec.after !== "string" || !Object.hasOwn(out, spec.after)) {
        throw new Error("valle:compute:sequence stage `" + label + "` references unknown or forward `after` label `" + spec.after + "`");
      }
      const previous = out[spec.after];
      start = previous.start + previous.duration + timeValue("delay", spec.delay)
        - timeValue("overlap", spec.overlap);
    } else if (index === 0) {
      start = 0;
    } else {
      throw new Error("valle:compute:sequence stage `" + label + "` needs explicit at or after");
    }
    if (!Number.isFinite(start) || start < 0) {
      throw new Error("valle:compute:sequence stage `" + label + "` starts before zero");
    }
    out[label] = { __valleType: "sequenceStage", unit: sequenceUnit,
      start, duration: duration.value };
  }
  return out;
};
const defineRepeater = (options = {}) => {
  if (options === null || typeof options !== "object" || Array.isArray(options)) {
    throw new Error("valle:compute:defineRepeater expects an options object");
  }
  const allowed = new Set(["count", "keyPrefix"]);
  for (const key of Object.keys(options)) {
    if (!allowed.has(key)) {
      throw new Error("valle:compute:defineRepeater has unknown option `" + key + "`");
    }
  }
  const count = options.count;
  if (!Number.isInteger(count) || count < 1 || count > 2048) {
    throw new Error("valle:compute:defineRepeater count must be an integer in 1..=2048");
  }
  const keyPrefix = options.keyPrefix ?? "copy";
  if (typeof keyPrefix !== "string" || keyPrefix.length === 0 || keyPrefix.length > 64) {
    throw new Error("valle:compute:defineRepeater keyPrefix must be a non-empty string of at most 64 characters");
  }
  const out = [];
  for (let index = 0; index < count; index += 1) {
    out.push({
      index,
      count,
      progress: count === 1 ? 0 : index / (count - 1),
      key: keyPrefix + "-" + index,
    });
  }
  return out;
};
const defineLayoutStates = (input) => {
  if (input === null || typeof input !== "object" || Array.isArray(input)) {
    throw new Error("valle:compute:defineLayoutStates expects an object of named states");
  }
  const entries = Object.entries(input);
  if (entries.length < 2 || entries.length > 16) {
    throw new Error("valle:compute:defineLayoutStates needs 2..=16 named states");
  }
  let ids = null;
  for (const [stateName, state] of entries) {
    if (state === null || typeof state !== "object" || Array.isArray(state)) {
      throw new Error("valle:compute:layout state `" + stateName + "` must be an object of layoutId Rect values");
    }
    const stateIds = Object.keys(state).sort();
    if (stateIds.length < 1 || stateIds.length > 512) {
      throw new Error("valle:compute:layout state `" + stateName + "` needs 1..=512 layoutId entries");
    }
    if (ids === null) {
      ids = stateIds;
    } else if (JSON.stringify(ids) !== JSON.stringify(stateIds)) {
      throw new Error("valle:compute:every layout state must declare the exact same layoutId set");
    }
    for (const id of stateIds) {
      const value = state[id];
      if (!value || value.__valleType !== "rect" ||
          !Number.isFinite(value.x) || !Number.isFinite(value.y) ||
          !Number.isFinite(value.width) || !Number.isFinite(value.height) ||
          value.width <= 0 || value.height <= 0) {
        throw new Error("valle:compute:layout state `" + stateName + "` entry `" + id +
          "` must be rect(x, y, positiveWidth, positiveHeight)");
      }
    }
  }
  return { __valleType: "layoutStates", states: input, ids };
};
const optionalFrames = (options = {}) => __control("optionalFrames", options);
const spanCue = (options = {}) => __control("spanCue", options);
const asset = (options = {}) => ({ kind: "asset", assetKind: options.kind, required: options.required ?? false });
