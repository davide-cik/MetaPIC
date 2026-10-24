# Componenti di terze parti

MetaPIC include i seguenti componenti open source principali, ciascuno distribuito con la propria licenza.

| Componente | Uso | Licenza |
|---|---|---|
| Tauri | Framework applicativo | MIT o Apache-2.0 |
| image | Decodifica JPEG; PNG, TIFF, GIF, BMP, ICO, QOI | MIT o Apache-2.0 |
| mozjpeg (via mozjpeg / mozjpeg-sys) | Codifica JPEG | IJG, BSD-3-Clause, Zlib |
| libwebp (via webp / libwebp-sys) | Codifica WebP | BSD-3-Clause (libwebp), MIT (binding) |
| ravif / rav1e | Codifica AVIF | BSD-3-Clause / BSD-2-Clause |
| avif-decode / rav1d | Decodifica AVIF | BSD-3-Clause / BSD-2-Clause |
| jxl-oxide | Decodifica JPEG XL | MIT o Apache-2.0 |
| zune-jpegxl | Codifica JPEG XL | MIT, Apache-2.0 o Zlib |
| little_exif | Lettura e scrittura EXIF | MIT o Apache-2.0 |

This software is based in part on the work of the Independent JPEG Group.

L'elenco completo delle dipendenze e delle relative licenze è ricavabile con `cargo tree` e `npm ls`.

JPEG, PNG, WebP, AVIF, JPEG XL, HEIC, TIFF, GIF e gli altri nomi di formato citati sono marchi o standard dei rispettivi titolari.
