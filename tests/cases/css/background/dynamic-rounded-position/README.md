# dynamic-rounded-position

The selected rows from `css/background/rounded-position` are [0, 2, 3, 5, 1, 0, 5, 2].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.
