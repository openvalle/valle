# dynamic-nested-decorations

The selected rows from `css/text/nested-decorations` are [0, 1, 2, 3, 1, 0, 3, 0].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.
