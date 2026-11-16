# dynamic-order-clipping

The selected rows from `css/transform/order-clipping` are [1, 3, 5, 7, 9, 11, 1, 7].
They are placed at the same origin with the same node key, including repeated
prior states. The static browser controls establish the expected appearance;
this case checks time-varying style invalidation and visit-order independence.

Only the clipped control rectangle is copied; the surrounding white margin excludes overflow from neighboring static controls.
