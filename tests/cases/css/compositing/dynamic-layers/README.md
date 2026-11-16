# dynamic-layers

Eight states change opacity, filter, transform, clip, mask, display and visibility around nested backdrop/blend layers. Zero opacity, display:none and visibility:hidden must produce identical background-only frames.

All eight frames are retained as numbered baselines. The case runner also compares
fresh loads with forward, reverse and alternating seeks, including checker state.
The WebGPU player repeats the same states in nonmonotonic order against native frames.
