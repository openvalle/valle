# dynamic-breaking

The selected rows from `css/text/breaking-products` are [0, 2, 36, 54, 72, 107, 0, 2].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.
