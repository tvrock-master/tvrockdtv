# Third-party licenses and source notices

TvRockDTV's own code is licensed under the MIT license in `LICENSE`. Third-party code and data retain their own terms; the project's MIT license does not replace them. Distribute `LICENSE`, this document, and the entire `licenses/` directory with each DLL.

## Rust dependencies

`licenses/Rust.txt` identifies every resolved Rust dependency for Windows x64/x86 by name and version, links to its published source, and reproduces its supplied license, copyright, and notice files. It includes indirect dependencies and build/proc-macro dependencies.

These include Tokio, tokio-tungstenite/tungstenite, futures, Serde/serde_json, url/idna, Windows bindings, and their dependencies. Most use MIT and/or Apache 2.0. ICU4X and related Unicode components use Unicode License v3; `unicode-ident` additionally requires Unicode License v3 alongside its MIT/Apache terms. The complete upstream texts are in `licenses/Rust.txt`.

Regenerate the notices after dependency updates with `./tools/write-license-notices.ps1`, after fetching/building the locked dependencies. `build.ps1` regenerates notices directly into its output directory from the actual resolved dependency graph. The checked-in notices describe the dependency versions available when last generated.

On Windows, `native-tls` uses Windows Schannel. OpenSSL is not linked by this project's Windows configuration; its presence in the cross-platform lockfile alone does not mean it is included in the distributed DLL.

## Static SRT dependencies

TvRockDTV links the following unmodified upstream libraries statically. No SRT or cryptographic runtime DLL is required. Sources and SHA-256 checksums are pinned in `native/dependencies.json`; the build verifies the archives before extraction.

| Component | Version | Source | License |
| --- | --- | --- | --- |
| SRT | 1.5.4 | https://github.com/Haivision/srt/tree/v1.5.4 | Mozilla Public License 2.0 |
| Mbed TLS | 3.6.5 | https://github.com/Mbed-TLS/mbedtls/releases/tag/mbedtls-3.6.5 | Apache License 2.0 option of the dual license |

SRT copyright: Haivision Systems Inc. and contributors. Mbed TLS and bundled cryptographic implementations retain the copyright notices in their source files. These components are not covered by this project's MIT license.

The full licenses are in `licenses/SRT.txt` and `licenses/MbedTLS.txt`. SRT also incorporates UDT-derived code with University of Illinois BSD notices and other embedded notices; these are reproduced in `licenses/SRT-THIRD-PARTY.txt`. Embedded copyright and license notices from Mbed TLS are in `licenses/MbedTLS-THIRD-PARTY.txt`. These files retain notices from the libraries' source/header directories, including optional source files that may not be compiled in this configuration.

SRT source is available under MPL 2.0, including when the library is used without modification. Recipients can obtain the exact unmodified source used here from:

- SRT 1.5.4 source archive: https://codeload.github.com/Haivision/srt/zip/refs/tags/v1.5.4
- Mbed TLS 3.6.5 source archive: https://github.com/Mbed-TLS/mbedtls/releases/download/mbedtls-3.6.5/mbedtls-3.6.5.tar.bz2

Archive SHA-256 checksums are recorded in `native/dependencies.json`. Preserve MPL notices. If MPL-covered files are modified and the resulting binary is distributed, make the corresponding modified covered source available under MPL 2.0 and update this source notice to describe how recipients can obtain it. A link to the unmodified upstream source does not cover local modifications.

## Imported character mappings and compatibility interfaces

`src/tables.rs` contains character mappings imported from the original `AribString.cpp` using `tools/import_tables.py`. The compatibility interfaces also derive from the original TvRockDTV interfaces. The original source revision, copyright holder notices, and applicable redistribution terms are not identified in the source material currently recorded in this repository. They cannot be reconstructed reliably from a filename or an interface name.

This document does not grant an MIT license to those third-party portions. Their provenance and original license notices still need to be verified against the original source distribution before treating the project's licensing review as complete. Preserve and include any applicable original notices once identified.
