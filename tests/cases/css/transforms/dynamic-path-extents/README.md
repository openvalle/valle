# dynamic-path-extents

The selected rows from `css/transforms/path-extents` are [0, 2, 4, 8, 12, 15, 0, 8].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.
