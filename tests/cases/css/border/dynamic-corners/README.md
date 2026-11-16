# dynamic-corners

The selected rows from `css/border/unequal-corners` are [0, 3, 9, 15, 23, 0, 15, 3].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.
