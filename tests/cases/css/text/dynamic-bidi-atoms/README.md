# dynamic-bidi-atoms

The selected rows from `css/text/bidi-inline-atoms` are [0, 3, 6, 9, 12, 17, 0, 9].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.
