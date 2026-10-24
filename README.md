# MetaPIC

Versione: **0.2.4**

App desktop (Windows, in futuro macOS) per convertire immagini nei formati moderni, in batch, con controllo di qualità, ridimensionamento e gestione dei metadati EXIF, più un contagocce che preleva colori da tutto lo schermo.

Repository: https://github.com/davide-cik/MetaPIC

Stack: [Tauri 2](https://tauri.app) + Rust (backend) e TypeScript senza framework (frontend).

## Formati

| Formato  | Lettura | Scrittura | EXIF in conversione | Modifica EXIF sul file |
|----------|:-------:|:---------:|:-------------------:|:----------------------:|
| JPEG     | ✓ | ✓ qualità (mozjpeg) | ✓ | ✓ |
| PNG      | ✓ | ✓ lossless | ✓ | ✓ |
| WebP     | ✓ | ✓ qualità / lossless | ✓ | ✓ |
| AVIF     | ✓ ¹ | ✓ qualità / velocità | ✓ | — |
| JPEG XL  | ✓ | ✓ solo lossless | ✓ | ✓ |
| TIFF     | ✓ | ✓ | ✓ | ✓ |
| GIF, BMP, ICO, QOI | ✓ | ✓ | — | — |
| HEIC     | — | — | | |

¹ Richiede NASM per compilare (vedi sotto); senza, la build esclude la lettura AVIF.

L'orientamento EXIF viene applicato ai pixel, e nel file convertito il tag Orientation viene riportato a 1.

## Contagocce

La scheda **Colori** preleva un colore da qualsiasi punto dello schermo, dal vivo, su tutti i monitor collegati e da qualsiasi finestra (siti, icone, video). MetaPIC si nasconde e una lente d'ingrandimento segue il puntatore mostrando il colore reale sotto di esso; il clic restituisce il pixel esatto in HEX, RGB, HSL e CMYK, con cronologia dei colori.

Come funziona: su ogni monitor si apre una finestra trasparente che intercetta il mouse ed è esclusa dalla cattura dello schermo (`WDA_EXCLUDEFROMCAPTURE`), così ogni lettura mostra il desktop vero e non la sovrapposizione.

Comandi: clic (o Invio / Spazio) per prelevare, frecce per spostare il puntatore di un pixel (Maiusc: 10 pixel), Esc o clic destro per annullare. Funziona sul desktop virtuale attivo: Windows non disegna quelli nascosti.

## Sviluppo

Requisiti: Rust (stable, MSVC), Visual Studio Build Tools con C++, Node 20+, NASM.

```bash
npm install
npm run tauri dev
```

Senza NASM: `npm run tauri dev -- -- --no-default-features`.

Test della pipeline (genera una foto con EXIF, la converte in tutti i formati e verifica dimensioni e metadati):

```bash
cd src-tauri && cargo test --release --lib
```

Versione: ogni commit incrementa la patch in automatico (hook `.githooks/pre-commit`, attivato da `npm install`). Per un salto manuale: `npm run bump-version -- 0.3.0`.

Build dell'installer: `npm run tauri build` (produce `.msi` / `.exe` in `src-tauri/target/release/bundle`).

## Licenza

MIT, © 2026 Davide Giansoldati / Content is king Srl ([davide@contentisking.guru](mailto:davide@contentisking.guru)). Vedi [LICENSE](LICENSE) e [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

## Struttura

- `src-tauri/src/codec.rs`: riconoscimento del formato dai byte, decodifica, codifica
- `src-tauri/src/meta.rs`: lettura, filtro (GPS), modifica e scrittura EXIF
- `src-tauri/src/heif.rs`: estrazione dell'item Exif da AVIF/HEIC
- `src-tauri/src/convert.rs`: pipeline per singolo file (orientamento → resize → codifica → metadati → salvataggio)
- `src-tauri/src/picker.rs`: contagocce dal vivo: sovrapposizioni trasparenti per monitor e lettura dei pixel sotto il puntatore
- `src-tauri/src/lib.rs`: comandi Tauri (conversione parallela con eventi di avanzamento, editor dei metadati)
- `src/main.ts`: interfaccia principale
- `picker.html`, `src/picker.ts`: sovrapposizione del contagocce con lente d'ingrandimento

## Da fare

- HEIC in lettura (libheif) e JPEG XL lossy (libjxl): richiedono librerie C con CMake
- Modifica in-place degli EXIF negli AVIF
- CLI che riusa lo stesso core
