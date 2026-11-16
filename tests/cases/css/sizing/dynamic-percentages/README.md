# dynamic-percentages

The selected rows from `css/sizing/indefinite-percentages` are [0, 2, 4, 8, 10, 0, 8, 2].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.
