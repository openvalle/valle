import type { CanvasKit, Canvas, Paint, Shader, SkPicture } from "canvaskit-wasm";

type Matrix = [number, number, number, number, number, number];
interface Gradient {
  matrix: Matrix;
  spread: string;
  stops: Array<{ offset: number; rgb: number[]; opacity: number }>;
}
type SvgPaint =
  | { kind: "color"; rgb: number[] }
  | { kind: "linear"; start: number[]; end: number[]; gradient: Gradient }
  | { kind: "radial"; center: number[]; radius: number; focal: number[]; gradient: Gradient }
  | { kind: "pattern"; ops: Op[]; matrix: Matrix; width: number; height: number };
type Op =
  | { kind: "transform"; matrix: Matrix }
  | { kind: "clip"; path: string; evenodd: boolean }
  | { kind: "blend"; mode: string }
  | { kind: "opacity"; opacity: number }
  | { kind: "mask"; ops: Op[]; luminance: boolean }
  | { kind: "pop" }
  | {
      kind: "draw";
      path: string;
      fill: { paint: SvgPaint; opacity: number; evenodd: boolean } | null;
      stroke: {
        paint: SvgPaint;
        opacity: number;
        width: number;
        miter: number;
        cap: string;
        join: string;
        dash: [number[], number] | null;
      } | null;
    }
  | { kind: "raster"; rgba: number[]; width: number; height: number; rect: number[] };
export interface SvgDocument {
  width: number;
  height: number;
  ops: Op[];
}
const matrix = ([a, b, c, d, e, f]: Matrix) => [a, c, e, b, d, f, 0, 0, 1];
export function isSvg(bytes: Uint8Array): boolean {
  let i = bytes[0] === 239 && bytes[1] === 187 && bytes[2] === 191 ? 3 : 0;
  while (i < bytes.length && [9, 10, 13, 32].includes(bytes[i])) i++;
  return bytes[i] === 60;
}
export function recordSvg(
  ck: CanvasKit,
  doc: SvgDocument,
  normalize?: (source: Shader) => Shader,
): SkPicture {
  const record = (ops: Op[], width: number, height: number): SkPicture => {
    const recorder = new ck.PictureRecorder();
    try {
      const canvas = recorder.beginRecording(ck.XYWHRect(0, 0, width, height));
      canvas.clipRect(ck.XYWHRect(0, 0, width, height), ck.ClipOp.Intersect, true);
      canvas.saveLayer(undefined, ck.XYWHRect(0, 0, width, height));
      draw(canvas, ops);
      canvas.restore();
      return recorder.finishRecordingAsPicture();
    } finally {
      recorder.delete();
    }
  };
  const paint = (src: SvgPaint, opacity: number): Paint => {
    const p = new ck.Paint();
    p.setAntiAlias(true);
    p.setAlphaf(opacity);
    if (src.kind === "color" && !normalize) {
      p.setColor(ck.Color(src.rgb[0], src.rgb[1], src.rgb[2], opacity));
      return p;
    }
    try {
      let shader;
      if (src.kind === "color") {
        shader = ck.Shader.MakeColor(
          ck.Color4f(src.rgb[0] / 255, src.rgb[1] / 255, src.rgb[2] / 255, 1),
          ck.ColorSpace.SRGB,
        );
      } else if (src.kind === "pattern") {
        const picture = record(src.ops, src.width, src.height);
        try {
          shader = picture.makeShader(
            ck.TileMode.Repeat,
            ck.TileMode.Repeat,
            ck.FilterMode.Linear,
            matrix(src.matrix),
            ck.XYWHRect(0, 0, src.width, src.height),
          );
        } finally {
          picture.delete();
        }
      } else {
        const g = src.gradient,
          colors = g.stops.map((s) => ck.Color(s.rgb[0], s.rgb[1], s.rgb[2], s.opacity)),
          offsets = g.stops.map((s) => s.offset);
        const tile =
          g.spread === "repeat"
            ? ck.TileMode.Repeat
            : g.spread === "mirror"
              ? ck.TileMode.Mirror
              : ck.TileMode.Clamp;
        shader =
          src.kind === "linear"
            ? ck.Shader.MakeLinearGradient(
                src.start,
                src.end,
                colors,
                offsets,
                tile,
                matrix(g.matrix),
              )
            : ck.Shader.MakeTwoPointConicalGradient(
                src.focal,
                0,
                src.center,
                src.radius,
                colors,
                offsets,
                tile,
                matrix(g.matrix),
              );
      }
      if (!shader) throw new Error("SVG paint shader failed");
      if (normalize && src.kind !== "pattern") {
        const input = shader;
        try {
          shader = normalize(input);
        } finally {
          input.delete();
        }
      }
      try {
        p.setShader(shader);
      } finally {
        shader.delete();
      }
      return p;
    } catch (error) {
      p.delete();
      throw error;
    }
  };
  const draw = (canvas: Canvas, ops: Op[]): void => {
    const stack: Array<{ ops: Op[]; luminance: boolean } | null> = [];
    for (const op of ops) {
      switch (op.kind) {
        case "transform":
          canvas.save();
          canvas.concat(matrix(op.matrix));
          stack.push(null);
          break;
        case "clip": {
          const path = op.path ? ck.Path.MakeFromSVGString(op.path) : new ck.Path();
          if (!path) throw new Error("invalid SVG clip");
          try {
            path.setFillType(op.evenodd ? ck.FillType.EvenOdd : ck.FillType.Winding);
            canvas.save();
            canvas.clipPath(path, ck.ClipOp.Intersect, true);
            stack.push(null);
          } finally {
            path.delete();
          }
          break;
        }
        case "opacity": {
          const p = new ck.Paint();
          try {
            p.setAlphaf(op.opacity);
            canvas.saveLayer(p);
            stack.push(null);
          } finally {
            p.delete();
          }
          break;
        }
        case "blend": {
          const p = new ck.Paint();
          try {
            const name =
              op.mode === "Normal" ? "SrcOver" : op.mode === "PlusLighter" ? "Plus" : op.mode;
            const blend = ck.BlendMode[name as keyof typeof ck.BlendMode];
            if (!blend || Array.isArray(blend)) throw new Error(`unsupported SVG blend ${op.mode}`);
            p.setBlendMode(blend);
            canvas.saveLayer(p);
            stack.push(null);
          } finally {
            p.delete();
          }
          break;
        }
        case "mask":
          canvas.saveLayer();
          stack.push(op);
          break;
        case "pop": {
          if (!stack.length) throw new Error("SVG group underflow");
          const mask = stack.pop();
          if (mask) {
            const p = new ck.Paint();
            try {
              p.setBlendMode(ck.BlendMode.DstIn);
              if (mask.luminance) {
                const f = ck.ColorFilter.MakeLuma();
                try {
                  p.setColorFilter(f);
                } finally {
                  f.delete();
                }
              }
              canvas.saveLayer(p);
              draw(canvas, mask.ops);
              canvas.restore();
            } finally {
              p.delete();
            }
          }
          canvas.restore();
          break;
        }
        case "draw": {
          const path = op.path ? ck.Path.MakeFromSVGString(op.path) : new ck.Path();
          if (!path) throw new Error("invalid SVG path");
          try {
            if (op.fill) {
              path.setFillType(op.fill.evenodd ? ck.FillType.EvenOdd : ck.FillType.Winding);
              const p = paint(op.fill.paint, op.fill.opacity);
              try {
                canvas.drawPath(path, p);
              } finally {
                p.delete();
              }
            }
            if (op.stroke) {
              const s = op.stroke,
                p = paint(s.paint, s.opacity);
              try {
                p.setStyle(ck.PaintStyle.Stroke);
                p.setStrokeWidth(s.width);
                p.setStrokeMiter(s.miter);
                p.setStrokeCap(
                  s.cap === "Round"
                    ? ck.StrokeCap.Round
                    : s.cap === "Square"
                      ? ck.StrokeCap.Square
                      : ck.StrokeCap.Butt,
                );
                p.setStrokeJoin(
                  s.join === "Round"
                    ? ck.StrokeJoin.Round
                    : s.join === "Bevel"
                      ? ck.StrokeJoin.Bevel
                      : ck.StrokeJoin.Miter,
                );
                if (s.dash) {
                  const effect = ck.PathEffect.MakeDash(s.dash[0], s.dash[1]);
                  if (!effect) throw new Error("invalid SVG dash");
                  try {
                    p.setPathEffect(effect);
                  } finally {
                    effect.delete();
                  }
                }
                canvas.drawPath(path, p);
              } finally {
                p.delete();
              }
            }
          } finally {
            path.delete();
          }
          break;
        }
        case "raster": {
          const image = ck.MakeImage(
            {
              width: op.width,
              height: op.height,
              colorType: ck.ColorType.RGBA_8888,
              alphaType: ck.AlphaType.Unpremul,
              colorSpace: ck.ColorSpace.SRGB,
            },
            new Uint8Array(op.rgba),
            op.width * 4,
          );
          if (!image) throw new Error("SVG raster fallback failed");
          const p = new ck.Paint();
          try {
            if (normalize) {
              const input = image.makeShaderOptions(
                ck.TileMode.Clamp,
                ck.TileMode.Clamp,
                ck.FilterMode.Linear,
                ck.MipmapMode.None,
              );
              const shader = normalize(input);
              try {
                p.setShader(shader);
                canvas.save();
                canvas.translate(op.rect[0], op.rect[1]);
                canvas.scale(op.rect[2] / op.width, op.rect[3] / op.height);
                canvas.drawRect(ck.XYWHRect(0, 0, op.width, op.height), p);
                canvas.restore();
              } finally {
                shader.delete();
                input.delete();
              }
            } else
              canvas.drawImageRectOptions(
                image,
                ck.XYWHRect(0, 0, op.width, op.height),
                ck.XYWHRect(op.rect[0], op.rect[1], op.rect[2], op.rect[3]),
                ck.FilterMode.Linear,
                ck.MipmapMode.None,
                p,
              );
          } finally {
            p.delete();
            image.delete();
          }
          break;
        }
      }
    }
    if (stack.length) throw new Error("SVG groups are unbalanced");
  };
  return record(doc.ops, doc.width, doc.height);
}
