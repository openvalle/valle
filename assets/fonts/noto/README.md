# Noto fonts

The COLRv1 emoji font is the unmodified `fonts/Noto-COLRv1.ttf` from
https://github.com/googlefonts/noto-emoji at revision
`8998f5dd683424a73e2314a8c1f1e359c19e8742`.

SHA-256: `0ae57fe58645638523ba35f388d93739d292539a9acb84df5700c81b1e1a28d2`.
Copyright 2022 Google Inc. (from the font name table).
The license is in `OFL-notoemoji.txt`.

OpenValle uses this font as the default `emoji` family in Native and Web.
The COLRv1 adapter covers this pinned font's solid paints, linear and radial
gradients, transforms, clipping, and SourceOver/SourceIn/SoftLight composition.
Other COLRv1 paint features are rejected explicitly; this is not a complete
implementation of every optional COLRv1 operation.

## Variable Noto Sans

`NotoSans-Variable.ttf` is the unmodified `ofl/notosans/NotoSans[wdth,wght].ttf`
from [google/fonts](https://github.com/google/fonts/tree/2984c575fdce412ee02b2baaba67672b9a9434d8/ofl/notosans),
revision `2984c575fdce412ee02b2baaba67672b9a9434d8`.
SHA-256: `bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d`.
Copyright 2022 The Noto Project Authors. The accompanying license is
`OFL-notosans-variable.txt` (SIL OFL 1.1).

This is the default Motion sans-serif face in Native and Web: `wght` spans
100–900 (default 400), and `wdth` spans 62.5–100 (default 100). It replaces the
five static sans faces in the default pack. Existing static files remain font
fixtures; they are not loaded alongside the variable default.

## Variable Noto Sans CJK SC

`NotoSansCJKsc-Variable.otf` is the unmodified `Sans/Variable/OTF/NotoSansCJKsc-VF.otf`
from [notofonts/noto-cjk](https://github.com/notofonts/noto-cjk/blob/f8d157532fbfaeda587e826d4cd5b21a49186f7c/Sans/Variable/OTF/NotoSansCJKsc-VF.otf),
revision `f8d157532fbfaeda587e826d4cd5b21a49186f7c`.
SHA-256: `2745e9681cb9d8a5c8901b62c9e1bd98c9c774365fc3b84dd467621013b51cd3`.
It uses the existing `OFL-notosanscjk.txt` license. The variable CJK face replaces
Regular in the default Native/Web font pack so mixed Chinese/Latin text can use
real bold glyphs and continuous `fontWeight` animation. The static Regular file
remains an explicit test fixture.
