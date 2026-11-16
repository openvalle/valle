# mixed-size-matrix

Full product: 3 writing modes × 2 orientations × 2 child sizes = 12.

The equal-size and horizontal controls distinguish central-baseline adjustment
from general font rasterization. This fixture isolated the missing mixed-size
vertical alignment; a unit test asserts the pinned-font 3.5px baseline delta.

Fonts are pinned. Browser images are compared and reviewed before selecting a
baseline. Rasterization allowances must not hide missing lines or shifted boxes.
