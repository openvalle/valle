# dynamic-three-layers

The selected rows from `css/mask/three-layer-composites` are [0, 1, 2, 3, 5, 7, 0, 3].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.
