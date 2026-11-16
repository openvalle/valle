# dynamic-backdrop-roots

The selected rows from `css/compositing/backdrop-roots` are [0, 3, 5, 7, 9, 11, 0, 7].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.
