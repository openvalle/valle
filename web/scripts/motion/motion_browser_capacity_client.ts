export {};

type RenderTiming = {
  totalMs: number;
  prepareMs?: number;
  submitAndFinishMs?: number;
};

const query = new URLSearchParams(location.search);
const mode = query.get('mode');
const count = Number(query.get('count'));
const cols = Math.sqrt(count);
const frames = 60;
const warmups = 5;
const cells = Array.from({ length: count }, (_, i) => {
  const c = i % cols;
  const r = Math.floor(i / cols);
  const dx = c - cols / 2;
  const dy = r - cols / 2;
  return { x: 8 + c * (624 / cols), y: 6 + r * (348 / cols), d: Math.sqrt(dx * dx + dy * dy) / cols };
});

function percentile(values: number[], fraction: number) {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.floor(fraction * (sorted.length - 1))];
}

function gpuIdentity(gl: WebGL2RenderingContext) {
  const ext = gl.getExtension('WEBGL_debug_renderer_info');
  return ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
}

function shader(gl: WebGL2RenderingContext, type: number, source: string) {
  const value = gl.createShader(type);
  if (!value) throw new Error("Could not create WebGL shader");
  gl.shaderSource(value, source);
  gl.compileShader(value);
  if (!gl.getShaderParameter(value, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(value) ?? "WebGL shader compilation failed");
  return value;
}

function webglRenderer() {
  const canvas = document.createElement('canvas');
  canvas.width = 640;
  canvas.height = 360;
  document.body.append(canvas);
  const context = canvas.getContext('webgl2', { alpha: false, antialias: true, preserveDrawingBuffer: true });
  if (!context) throw new Error('WebGL2 unavailable');
  const gl = context;
  const program = gl.createProgram();
  if (!program) throw new Error("Could not create WebGL program");
  gl.attachShader(program, shader(gl, gl.VERTEX_SHADER, `#version 300 es
    in vec2 corner; in vec3 fixedData; in vec3 frameData;
    out float vAlpha;
    void main() {
      vec2 local = corner - vec2(2.0);
      vec2 rotated = vec2(local.x * frameData.x - local.y * frameData.y,
                          local.x * frameData.y + local.y * frameData.x);
      vec2 pixel = fixedData.xy + rotated + vec2(2.0);
      gl_Position = vec4(pixel.x / 640.0 * 2.0 - 1.0, 1.0 - pixel.y / 360.0 * 2.0, 0.0, 1.0);
      vAlpha = frameData.z;
    }`));
  gl.attachShader(program, shader(gl, gl.FRAGMENT_SHADER, `#version 300 es
    precision highp float; in float vAlpha; out vec4 outColor;
    void main() { outColor = vec4(vec3(139.0, 123.0, 255.0) / 255.0 * vAlpha, vAlpha); }`));
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(program) ?? "WebGL program linking failed");
  gl.useProgram(program);
  const vao = gl.createVertexArray();
  gl.bindVertexArray(vao);
  function attrib(name: string, data: Float32Array | number, size: number, usage: number, divisor: number) {
    const location = gl.getAttribLocation(program, name);
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    if (typeof data === 'number') gl.bufferData(gl.ARRAY_BUFFER, data, usage);
    else gl.bufferData(gl.ARRAY_BUFFER, data, usage);
    gl.enableVertexAttribArray(location);
    gl.vertexAttribPointer(location, size, gl.FLOAT, false, 0, 0);
    gl.vertexAttribDivisor(location, divisor);
    return buffer;
  }
  attrib('corner', new Float32Array([0, 0, 4, 0, 0, 4, 4, 4]), 2, gl.STATIC_DRAW, 0);
  const fixed = new Float32Array(count * 3);
  for (let i = 0; i < count; i++) {
    fixed[i * 3] = cells[i].x;
    fixed[i * 3 + 1] = cells[i].y;
    fixed[i * 3 + 2] = cells[i].d;
  }
  attrib('fixedData', fixed, 3, gl.STATIC_DRAW, 1);
  const dynamic = new Float32Array(count * 3);
  const dynamicBuffer = attrib('frameData', dynamic.byteLength, 3, gl.DYNAMIC_DRAW, 1);
  gl.viewport(0, 0, 640, 360);
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  const gpu = gpuIdentity(gl);
  function render(frame: number): RenderTiming {
    const t = frame / 30;
    const began = performance.now();
    for (let i = 0; i < count; i++) {
      const angle = (t * 90 + cells[i].d * 180) * Math.PI / 180;
      dynamic[i * 3] = Math.cos(angle);
      dynamic[i * 3 + 1] = Math.sin(angle);
      dynamic[i * 3 + 2] = Math.max(0, Math.min(1, t * 2 - cells[i].d));
    }
    const prepareMs = performance.now() - began;
    gl.bindBuffer(gl.ARRAY_BUFFER, dynamicBuffer);
    gl.bufferSubData(gl.ARRAY_BUFFER, 0, dynamic);
    gl.clearColor(16 / 255, 16 / 255, 16 / 255, 1);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.drawArraysInstanced(gl.TRIANGLE_STRIP, 0, 4, count);
    gl.finish();
    const totalMs = performance.now() - began;
    return { prepareMs, submitAndFinishMs: totalMs - prepareMs, totalMs };
  }
  function inspect() {
    const center = new Uint8Array(4);
    const corner = new Uint8Array(4);
    gl.readPixels(320, 180, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, center);
    gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, corner);
    return { center: [...center], corner: [...corner], glError: gl.getError() };
  }
  return { render, gpu, inspect };
}

function domRenderer() {
  const root = document.createElement('div');
  root.style.cssText = 'position:relative;width:640px;height:360px;overflow:hidden;background:#101010;contain:layout paint style';
  const elements = cells.map((cell) => {
    const el = document.createElement('div');
    el.style.cssText = `position:absolute;left:${cell.x}px;top:${cell.y}px;width:4px;height:4px;background:#8b7bff;transform-origin:2px 2px`;
    root.append(el);
    return el;
  });
  document.body.append(root);
  function render(frame: number): RenderTiming {
    const t = frame / 30;
    const began = performance.now();
    for (let i = 0; i < count; i++) {
      const cell = cells[i];
      elements[i].style.opacity = String(Math.max(0, Math.min(1, t * 2 - cell.d)));
      elements[i].style.transform = `rotate(${t * 90 + cell.d * 180}deg)`;
    }
    const totalMs = performance.now() - began;
    return { totalMs };
  }
  const gl = document.createElement('canvas').getContext('webgl2');
  return { render, gpu: gl ? gpuIdentity(gl) : 'no webgl2', inspect: () => ({ elements: elements.length }) };
}

async function main() {
  const renderer = mode === 'webgl' ? webglRenderer() : mode === 'dom' ? domRenderer() : null;
  if (!renderer) throw new Error('unknown mode');
  const total: number[] = [], prepare: number[] = [], submit: number[] = [], rafIntervals: number[] = [];
  let lastRaf: number | null = null;
  for (let i = -warmups; i < frames; i++) {
    const raf = await new Promise<number>(requestAnimationFrame);
    if (i >= 0 && lastRaf !== null) rafIntervals.push(raf - lastRaf);
    lastRaf = raf;
    const result = renderer.render(Math.max(0, i));
    if (i >= 0) {
      total.push(result.totalMs);
      if (result.prepareMs !== undefined) prepare.push(result.prepareMs);
      if (result.submitAndFinishMs !== undefined) submit.push(result.submitAndFinishMs);
    }
  }
  const report = {
    mode, count, width: 640, height: 360, frames,
    browser: navigator.userAgent, gpu: renderer.gpu,
    operationP50Ms: percentile(total, 0.5), operationP95Ms: percentile(total, 0.95),
    rafIntervalP50Ms: percentile(rafIntervals, 0.5), rafIntervalP95Ms: percentile(rafIntervals, 0.95),
    rafIntervalMaxMs: Math.max(...rafIntervals),
    probe: renderer.inspect(),
    prepareP50Ms: prepare.length ? percentile(prepare, 0.5) : null,
    submitAndFinishP50Ms: submit.length ? percentile(submit, 0.5) : null,
  };
  await fetch('/result', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(report) });
}
main().catch(async (error) => {
  await fetch('/result', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ error: String(error.stack || error) }) });
});
