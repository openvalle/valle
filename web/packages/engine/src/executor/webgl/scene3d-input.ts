/** Versioned, admitted CPU coverage input. All slices borrow the worker transfer buffer. */
export function decodeScene3dLightingInput(
  packet: Float32Array | undefined,
  width: number,
  height: number,
  meshes: number,
) {
  const rays = 8 + (width + height) * 3;
  if (
    !(packet instanceof Float32Array) ||
    packet.length < rays + 5 ||
    packet[0] !== 1 ||
    packet[1] !== width ||
    packet[2] !== height ||
    packet[3] !== meshes ||
    !Number.isSafeInteger(width) ||
    !Number.isSafeInteger(height) ||
    width < 1 ||
    height < 1 ||
    width > 4096 ||
    height > 4096 ||
    width * height > 8_388_608 ||
    !Number.isSafeInteger(meshes) ||
    meshes < 1 ||
    meshes > 8 ||
    !packet.subarray(4, 8).every(Number.isFinite) ||
    packet[7]! <= 0
  ) {
    throw new TypeError("Invalid Scene3D lighting input header");
  }
  const [x, y, w, h, count] = packet.subarray(rays, rays + 5) as Float32Array;
  if (
    ![x, y, w, h, count].every(Number.isSafeInteger) ||
    x! < 0 ||
    y! < 0 ||
    w! < 1 ||
    h! < 1 ||
    x! + w! > width ||
    y! + h! > height ||
    count! < 0 ||
    count! > w! * h! ||
    packet.length !== rays + 5 + count! * 8
  ) {
    throw new TypeError("Invalid Scene3D lighting input spans");
  }
  return {
    eye: packet.subarray(4, 7),
    depthScale: packet[7]!,
    columns: packet.subarray(8, 8 + width * 3),
    rows: packet.subarray(8 + width * 3, rays),
    x: x!,
    y: y!,
    width: w!,
    height: h!,
    count: count!,
    spans: packet.subarray(rays + 5),
  };
}
