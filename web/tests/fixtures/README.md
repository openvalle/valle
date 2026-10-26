# Browser media fixtures

`video-offset-bframes.mp4` is generated test data: 90 frames at 64 × 36 / 30 fps, red/green/blue
one-second segments, two B frames between reference frames, one-second GOPs, and a 3-second
presentation origin. Its explicit BT.709 metadata avoids different decoder defaults for untagged
YUV. The browser checks use it to verify frame selection, source-time rebasing, efficient sequential
decoding and keyframe thumbnails. It contains no audio or third-party material.

Regenerate from the repository root with FFmpeg (fixture preparation only):

```sh
ffmpeg -v error \
  -f lavfi -i 'color=c=0xe03030:s=64x36:r=30:d=1' \
  -f lavfi -i 'color=c=0x20c060:s=64x36:r=30:d=1' \
  -f lavfi -i 'color=c=0x3070e0:s=64x36:r=30:d=1' \
  -filter_complex '[0:v][1:v][2:v]concat=n=3:v=1:a=0,scale=out_color_matrix=bt709:in_color_matrix=bt601,setpts=PTS+3/TB[v]' \
  -map '[v]' -c:v libx264 -bf 2 -g 30 -keyint_min 30 -sc_threshold 0 \
  -pix_fmt yuv420p -colorspace bt709 -color_primaries bt709 -color_trc bt709 \
  -fps_mode passthrough -an -y web/tests/fixtures/video-offset-bframes.mp4
```

`prores-alpha.mov` is generated test data: 12 frames at 128 × 72 / 12 fps, with a moving
half-transparent orange box, an opaque green box and a transparent background. It exercises
the ProRes 4444 decoder, alpha preservation, HTTP range reads and repeated random seeks.
It contains no audio or third-party material.

Save this source as `alpha.motion.tsx`, then run
`valle motion render alpha.motion.tsx --codec prores4444 -o web/tests/fixtures/prores-alpha.mov`:

```tsx
export const composition = { width: 128, height: 72, fps: 12, duration: 1 };
export default function Alpha() {
  const time = useCurrentTime();
  return (
    <Scene>
      <View style={{ position: "absolute", left: time.seconds * 36 + 8, top: 8,
        width: 48, height: 48, backgroundColor: "#f0804080", borderRadius: 8 }} />
      <View style={{ position: "absolute", left: 80, top: 20,
        width: 32, height: 32, backgroundColor: "#20c0a0" }} />
    </Scene>
  );
}
```
