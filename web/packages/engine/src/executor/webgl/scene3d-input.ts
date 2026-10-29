export interface Scene3dGpuTexture {
  width: number;
  height: number;
  words: Uint32Array;
}
export interface Scene3dGpuShadow extends Scene3dGpuTexture {
  vertices: number;
  minimum: number;
  scale: number;
}
export interface Scene3dGpuPacket {
  bytes: Uint8Array;
  width: number;
  height: number;
  vertices: number;
  stride: number;
  lit: boolean;
  blend: boolean;
  background: boolean;
  depthScale: number;
  focus: number;
  radius: number;
  bounds: [number, number, number, number];
  vertexData: Scene3dGpuTexture;
  shading: Scene3dGpuTexture;
  references: Scene3dGpuTexture;
  tiles: Scene3dGpuTexture;
  atlas: { width: number; height: number; bytes: number; key: string };
  shadows: Scene3dGpuShadow[];
  shadowKey: string;
  mesh?: { width: number; height: number; bytes: number; key: string };
}

/** Borrow admitted triangle/uniform uploads. The immutable atlas travels once, separately. */
export function decodeScene3dGpuPacket(bytes: Uint8Array): Scene3dGpuPacket {
  const invalid = (part: string): never => {
    throw new TypeError(`Invalid Scene3D GPU ${part}`);
  };
  if (
    !(bytes instanceof Uint8Array) ||
    bytes.byteOffset % 4 !== 0 ||
    bytes.byteLength < 256 ||
    bytes.byteLength % 16 !== 0
  )
    invalid("packet storage");
  const header = new Uint32Array(bytes.buffer, bytes.byteOffset, 64);
  const floats = new Float32Array(bytes.buffer, bytes.byteOffset, 64);
  const [magic, width, height, vertices, stride, flags] = Array.from(header) as [
    number,
    number,
    number,
    number,
    number,
    number,
  ];
  if (
    magic !== 0x34534756 ||
    !width ||
    !height ||
    width > 4096 ||
    height > 4096 ||
    width * height > 8_388_608 ||
    vertices % 3 !== 0 ||
    ![9, 13, 27].includes(stride) ||
    flags > 31 ||
    header[25] !== bytes.byteLength ||
    !Number.isFinite(floats[6]) ||
    floats[6]! <= 0 ||
    floats[6]! > 1 ||
    !Number.isFinite(floats[7]) ||
    floats[7]! <= 0 ||
    !Number.isFinite(floats[8]) ||
    floats[8]! < 0
  )
    invalid("header");
  let end = 256;
  const texture = (at: number): Scene3dGpuTexture => {
    const w = header[at]!,
      h = header[at + 1]!,
      offset = header[at + 2]!;
    const length = w * h * 16;
    if (!w || !h || w > 4096 || h > 4096 || offset !== end || offset + length > bytes.byteLength)
      invalid("texture span");
    end += length;
    return {
      width: w,
      height: h,
      words: new Uint32Array(bytes.buffer, bytes.byteOffset + offset, length / 4),
    };
  };
  const vertexData = texture(9),
    shading = texture(12),
    references = texture(18),
    tiles = texture(21);
  const lit = (flags & 1) !== 0,
    blend = (flags & 2) !== 0;
  const direct = (flags & 16) !== 0;
  if (
    (!direct && vertices * stride > vertexData.words.length) ||
    (direct && (!lit || blend || stride !== 13 || header[24] !== 0)) ||
    (lit ? stride === 9 : stride !== 9) ||
    (!lit && (flags & 10) !== 0)
  )
    invalid("vertex span");
  const tileCount = Math.ceil(width / 16) * Math.ceil(height / 16);
  if (header[27] !== (blend ? tileCount : 0) || (blend && tileCount * 4 > tiles.words.length))
    invalid("tile span");
  if (blend)
    for (let i = 0; i < tileCount; i++) {
      const start = tiles.words[i * 4]!,
        count = tiles.words[i * 4 + 1]!;
      if (start + count > references.words.length) invalid("tile references");
      for (let j = start; j < start + count; j++)
        if (references.words[j]! >= vertices / 3) invalid("triangle reference");
    }
  const shadows: Scene3dGpuShadow[] = [];
  if (header[24]! > 2) invalid("shadow count");
  for (let i = 0; i < header[24]!; i++) {
    const at = 48 + i * 8,
      data = texture(at + 1);
    const count = header[at]!,
      minimum = floats[at + 4]!,
      scale = floats[at + 5]!;
    if (
      count % 3 !== 0 ||
      count * 7 > data.words.length ||
      !Number.isFinite(minimum) ||
      !Number.isFinite(scale) ||
      scale <= 0
    )
      invalid("shadow data");
    shadows.push({ ...data, vertices: count, minimum, scale });
  }
  if (end !== bytes.byteLength) invalid("packet trailing bytes");
  const hex = (start: number, count: number) =>
    Array.from(bytes.subarray(start, start + count), (byte) =>
      byte.toString(16).padStart(2, "0"),
    ).join("");
  const atlas = { width: header[15]!, height: header[16]!, bytes: header[17]!, key: hex(112, 32) };
  const mesh = direct
    ? { width: header[48]!, height: header[49]!, bytes: header[50]!, key: hex(144, 32) }
    : undefined;
  if (
    mesh &&
    (!mesh.width ||
      !mesh.height ||
      mesh.width > 4096 ||
      mesh.height > 4096 ||
      mesh.bytes !== mesh.width * mesh.height * 16 ||
      vertices * 16 > mesh.bytes)
  )
    invalid("mesh extent");
  if (
    lit &&
    (!atlas.width ||
      !atlas.height ||
      atlas.width > 4096 ||
      atlas.height > 4096 ||
      atlas.bytes !== atlas.width * atlas.height * 16)
  )
    invalid("atlas extent");
  const bounds = Array.from(floats.subarray(44, 48)) as [number, number, number, number];
  if (!bounds.every(Number.isFinite) || bounds[0] > bounds[2] || bounds[1] > bounds[3])
    invalid("bounds");
  return {
    bytes,
    width,
    height,
    vertices,
    stride,
    lit,
    blend,
    background: (flags & 8) !== 0,
    depthScale: floats[6]!,
    focus: floats[7]!,
    radius: (flags & 4) !== 0 ? floats[8]! : 0,
    bounds,
    vertexData,
    shading,
    references,
    tiles,
    atlas,
    shadows,
    shadowKey: hex(144, 32),
    mesh,
  };
}
