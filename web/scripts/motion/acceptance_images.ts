/** Small image helpers for the Motion acceptance gallery; FFmpeg handles codecs. */
import { spawnSync } from "node:child_process";
import { join } from "node:path";

function ffmpeg(args: string[], input?: Uint8Array): Uint8Array {
  const result = spawnSync("ffmpeg", ["-v", "error", ...args], {
    input, maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error || result.status !== 0) {
    throw new Error(`ffmpeg failed: ${result.stderr.toString().slice(-2000)}`, { cause: result.error });
  }
  return result.stdout;
}

function image(width: number, height: number, color: readonly [number, number, number]): Uint8Array {
  const pixels = new Uint8Array(width * height * 4);
  for (let offset = 0; offset < pixels.length; offset += 4) {
    pixels[offset] = color[0]; pixels[offset + 1] = color[1];
    pixels[offset + 2] = color[2]; pixels[offset + 3] = 255;
  }
  return pixels;
}

function pixel(pixels: Uint8Array, width: number, x: number, y: number, color: readonly [number, number, number]) {
  const offset = (y * width + x) * 4;
  pixels[offset] = color[0]; pixels[offset + 1] = color[1]; pixels[offset + 2] = color[2];
}

function polygon(pixels: Uint8Array, width: number, height: number, points: [number, number][], color: readonly [number, number, number]) {
  const xs = points.map(point => point[0]);
  const ys = points.map(point => point[1]);
  for (let y = Math.max(0, Math.min(...ys)); y <= Math.min(height - 1, Math.max(...ys)); y++) {
    for (let x = Math.max(0, Math.min(...xs)); x <= Math.min(width - 1, Math.max(...xs)); x++) {
      let inside = false;
      for (let a = 0, b = points.length - 1; a < points.length; b = a++) {
        const first = points[a]!, second = points[b]!;
        if ((first[1] > y + 0.5) !== (second[1] > y + 0.5)
          && x + 0.5 < (second[0] - first[0]) * (y + 0.5 - first[1]) / (second[1] - first[1]) + first[0]) {
          inside = !inside;
        }
      }
      if (inside) pixel(pixels, width, x, y, color);
    }
  }
}

export function writeShowcaseTexture(path: string) {
  const width = 1024, height = 512;
  const pixels = image(width, height, [20, 108, 131]);
  for (let y = 0; y < height; y++) {
    const shade = 1 - 0.22 * Math.abs(2 * y / height - 1);
    for (let x = 0; x < width; x++) pixel(pixels, width, x, y,
      [Math.trunc(20 * shade), Math.trunc(108 * shade), Math.trunc(131 * shade)]);
  }
  for (let x = 0; x < width; x += 128) {
    for (let y = 0; y < height; y++) for (let dx = 0; dx < 4; dx++) {
      if (x + dx < width) pixel(pixels, width, x + dx, y, [128, 214, 215]);
    }
  }
  for (let y = 64; y < height; y += 64) {
    for (let dy = 0; dy < 4; dy++) for (let x = 0; x < width; x++) {
      if (y + dy < height) pixel(pixels, width, x, y + dy, [128, 214, 215]);
    }
  }
  polygon(pixels, width, height, [[260,145],[390,95],[488,130],[535,220],[455,268],
    [415,365],[330,405],[280,300],[210,245]], [241,182,76]);
  polygon(pixels, width, height, [[720,270],[810,230],[894,280],[855,370],[765,390]], [241,182,76]);
  for (let y = 185; y <= 223; y++) for (let x = 448; x <= 486; x++) {
    if (((x - 467) / 19) ** 2 + ((y - 204) / 19) ** 2 <= 1) {
      pixel(pixels, width, x, y, [236,91,66]);
    }
  }
  ffmpeg(["-y", "-f", "rawvideo", "-pixel_format", "rgba", "-video_size", `${width}x${height}`,
    "-i", "pipe:0", "-frames:v", "1", path], pixels);
}

const glyphs: Record<string, string[]> = {
  A:["01110","10001","10001","11111","10001","10001","10001"],
  B:["11110","10001","10001","11110","10001","10001","11110"],
  C:["01111","10000","10000","10000","10000","10000","01111"],
  D:["11110","10001","10001","10001","10001","10001","11110"],
  E:["11111","10000","10000","11110","10000","10000","11111"],
  F:["11111","10000","10000","11110","10000","10000","10000"],
  G:["01111","10000","10000","10111","10001","10001","01111"],
  H:["10001","10001","10001","11111","10001","10001","10001"],
  I:["11111","00100","00100","00100","00100","00100","11111"],
  "0":["01110","10001","10011","10101","11001","10001","01110"],
  "1":["00100","01100","00100","00100","00100","00100","01110"],
  "2":["01110","10001","00001","00010","00100","01000","11111"],
  "3":["11110","00001","00001","01110","00001","00001","11110"],
  "4":["00010","00110","01010","10010","11111","00010","00010"],
  "5":["11111","10000","10000","11110","00001","00001","11110"],
  "6":["01111","10000","10000","11110","10001","10001","01110"],
  "7":["11111","00001","00010","00100","01000","01000","01000"],
  "8":["01110","10001","10001","01110","10001","10001","01110"],
  "9":["01110","10001","10001","01111","00001","00001","11110"],
};

function label(pixels: Uint8Array, width: number, x: number, y: number, value: string) {
  for (let index = 0; index < value.length; index++) {
    const glyph = glyphs[value[index]!]!;
    for (let row = 0; row < 7; row++) for (let col = 0; col < 5; col++) {
      if (glyph[row]![col] !== "1") continue;
      for (let dy = 0; dy < 2; dy++) for (let dx = 0; dx < 2; dx++) {
        pixel(pixels, width, x + index * 14 + col * 2 + dx, y + row * 2 + dy, [255,255,255]);
      }
    }
  }
}

export function writeOverview(ids: string[], thumbnails: string, output: string) {
  const thumbWidth = 320, thumbHeight = 180, labelHeight = 32, columns = 6;
  const width = columns * thumbWidth, height = 6 * (thumbHeight + labelHeight);
  const pixels = image(width, height, [21,25,35]);
  ids.forEach((id, index) => {
    const thumbnail = ffmpeg(["-i", join(thumbnails, `${id}.jpg`), "-frames:v", "1",
      "-vf", `scale=${thumbWidth}:${thumbHeight}`, "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"]);
    if (thumbnail.length !== thumbWidth * thumbHeight * 4) throw new Error(`invalid thumbnail: ${id}`);
    const x = index % columns * thumbWidth;
    const y = Math.floor(index / columns) * (thumbHeight + labelHeight);
    for (let row = 0; row < thumbHeight; row++) {
      pixels.set(thumbnail.subarray(row * thumbWidth * 4, (row + 1) * thumbWidth * 4), ((y + row) * width + x) * 4);
    }
    label(pixels, width, x + 9, y + thumbHeight + 6, id);
  });
  ffmpeg(["-y", "-f", "rawvideo", "-pixel_format", "rgba", "-video_size", `${width}x${height}`,
    "-i", "pipe:0", "-frames:v", "1", "-q:v", "2", output], pixels);
}
