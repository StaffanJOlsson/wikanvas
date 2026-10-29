# Third-party software

Wikanvas is licensed under the MIT License (`LICENSE`).
It builds on the software below, each under its own licence. Every package Wikanvas ships is under, or offers, a licence
on Wikanvas's allow-list (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode or CC0; docs/PLAN.md rule R6). lightningcss, a
build-time tool, is MPL-2.0, which R6 allows for build-only tools (the owner's decision of 2026-10-28). One exception
is still open for the owner: the fonts inside the whiteboard engine (SIL OFL 1.1 and the Ubuntu Font Licence). Both
are described below. Nothing comes from AGPL or BSL projects, or from TipTap Pro or
Cloud.

This file is the record in the source tree, and the standalone CLI (`scripts/cli-dist.sh`) ships it next to the
binary. The image carries a generated list as well: `/licenses.txt`, shown in Settings → About, is written by the
`licenses` stage of `docker/Dockerfile` from `cargo metadata` and `pnpm licenses list --json --prod`, and also holds
the licence files every listed package ships (docs/ARCHITECTURE.md §13.2). It is always current; this file has to be
updated by hand when `Cargo.lock` or `web/pnpm-lock.yaml` changes (see "Updating this file").

## Code ported into Wikanvas

| Where | From | Licence | What |
|---|---|---|---|
| `crates/wikanvas-core/src/convert/pm.rs` | [`@tiptap/y-tiptap`](https://github.com/ueberdosis/tiptap/tree/main/packages/y-tiptap) 3.0.10, © 2019 Kevin Jahns and © 2025 Tiptap GmbH | MIT | The Y XmlFragment ↔ ProseMirror JSON mapping: `yXmlFragmentToProsemirrorJSON`, `createTypeFromElementNode` / `createTypeFromTextNodes` and `yattr2markname`, ported to Rust (ARCHITECTURE §6.4, D49). The file's header names the source. The package itself is also bundled into the web client (below), with its MIT licence file |
| `crates/wikanvas-core/src/rank.rs` | [`fractional-indexing`](https://github.com/rocicorp/fractional-indexing) (rocicorp), after David Greenspan's "Implementing Fractional Indexing" | CC0-1.0 | `generateKeyBetween` and `generateNKeysBetween` over a base62 alphabet, checked against the upstream test vectors (D6) |

No code was borrowed from GetPaper (PLAN rule R9) in milestone 1. Design values come from ARCHITECTURE §9.6, never from
Outline's source.

## Inkboard

- **What:** the whiteboard engine behind Wikanvas's boards. `inkboard-core` is compiled into `wikanvas-server`; the
  Inkboard web app is compiled to WebAssembly and served from `/inkboard/<build>/` inside Wikanvas's board frame.
- **Source:** `vendor/inkboard`, a git submodule of <https://github.com/VictorLetirand/inkboard> at commit `41e0295`
  (version 0.2.0-beta.3). Wikanvas never modifies the submodule: its changes are the patch series in `patches/inkboard/`
  (0001–0006, patchset 3), and every file a patch changes carries a "Changed for Wikanvas" notice line.
- **Licence:** MIT OR Apache-2.0. Copyright (c) 2026 The Inkboard contributors. The licence texts are
  `vendor/inkboard/LICENSE-MIT` and `vendor/inkboard/LICENSE-APACHE`; the image ships them in
  `/usr/share/doc/wikanvas/inkboard/`, and the engine's dist ships them next to the wasm.
- **Its own third-party components:** `vendor/inkboard/THIRD_PARTY_LICENSES.md` (libraries and fonts, with their
  licence texts) and `vendor/inkboard/crates/inkboard-app/assets/ICONS-LICENSES.txt` (the icon library: draw.io
  shapes, Apache-2.0; Simple Icons, CC0-1.0; Tabler Icons, MIT; Lucide, ISC; Material Design Icons, Apache-2.0). Both
  are part of the engine's dist (`web/dist-inkboard`, embedded in `wikanvas-server`) and are served at
  `/inkboard/<build>/THIRD_PARTY_LICENSES.md` and `/inkboard/<build>/ICONS-LICENSES.txt`, which Settings → About links.
  The image also ships `THIRD_PARTY_LICENSES.md` in `/usr/share/doc/wikanvas/inkboard/`. Inkboard's list covers all of
  its executables (desktop ones included); Wikanvas's wasm is the `web-glow,embed` build without `net`, a subset of it.
- **Fonts (open for the owner):** the engine embeds egui's default fonts (`epaint_default_fonts`: Hack, Ubuntu-Light,
  NotoEmoji, emoji-icon-font; MIT, the Ubuntu Font Licence 1.0 and SIL OFL 1.1), its board fonts (Noto Sans, Gelasio,
  Patrick Hand: SIL OFL 1.1; Hack Bold: MIT and the Bitstream Vera licence) and STIX Two Math (SIL OFL 1.1). These font
  licences are permissive for embedding and redistribution, but they are not on R6's list. The owner decides whether
  R6 also allows OFL-1.1 and the Ubuntu Font Licence for fonts; until then they ship as Inkboard ships them.

## Notes on the licences below

- **Choices.** Where a package offers several licences, Wikanvas uses it under MIT or Apache-2.0 when one is offered:
  `Unlicense OR MIT` (aho-corasick, memchr and others) as MIT, `Apache-2.0 OR BSL-1.0` (ryu) as Apache-2.0,
  `0BSD OR MIT OR Apache-2.0` (adler2) as MIT, and `MIT OR Apache-2.0 OR LGPL-2.1-or-later` (r-efi, other targets
  only) as MIT.
- **Conjunctions.** Every part of an `AND` expression is on the allow-list: ring (`Apache-2.0 AND ISC`), matchit
  (`MIT AND BSD-3-Clause`), unicode-ident (`(MIT OR Apache-2.0) AND Unicode-3.0`) and finl_unicode
  (`(MIT OR Apache-2.0) AND Unicode-DFS-2016`).
- **comrak** 0.59.0, the Markdown parser of the server (D49), is BSD-2-Clause. Its default features (a CLI and a C
  library) are off.
- **Unicode-3.0** covers the ICU data crates (`icu_*`, `zerovec`, `tinystr` and their kin, 18 crates).
- **Left out on purpose:** webpki-roots (CDLA-Permissive-2.0): ureq's `rustls` feature is never enabled, and the CLI
  uses the system CA bundle (D54); option-ext (MPL-2.0): rust-embed's `interpolate-folder-path` feature is off;
  OpenSSL, native-tls and aws-lc: no crate pulls them in.

## lightningcss (MPL-2.0, build time only): allowed

Vite 8.3.5, the web client's bundler, depends on **lightningcss 1.33.0** and its platform package
(`lightningcss-linux-x64-gnu` in the build container), both **MPL-2.0**. They run only while `pnpm --dir web build`
runs (in `scripts/dev.sh` or the image's `web` stage), and none of their code ends up in `web/dist`, so they are not
part of anything Wikanvas distributes. On 2026-10-28 the owner allowed MPL-2.0 for build-only tools (PLAN R6), rather
than re-pinning to Vite 7 with @vitejs/plugin-react 5 (ARCHITECTURE §16, risk 24).

## Container images

The images are not part of the source, but the compose project runs them:

- **The runtime base** `gcr.io/distroless/cc-debian13:nonroot` (D25) holds Debian packages under their own licences:
  libc6 (LGPL-2.1+), the GCC runtime libraries libgcc-s1, libstdc++6 and libgomp1 (GPL-3+ with the GCC Runtime
  Library Exception), libssl3t64 (Apache-2.0), libzstd1 (BSD-3-Clause or GPL-2), zlib1g (Zlib), ca-certificates (the
  Mozilla CA data under MPL-2.0), tzdata (public domain), base-files, netbase and media-types. Their copyright files
  are in the image under `/usr/share/doc/<package>/`. These are the operating system layer, which R6 (a rule for
  Wikanvas's own dependencies) does not cover; they are listed here so the owner can see them.
- **Caddy** `caddy:2.11.7` (D57), the HTTPS front, is Apache-2.0, on an Alpine base (musl, MIT; BusyBox, GPL-2.0).
  Wikanvas does not modify or redistribute it; compose pulls the official image.

## Rust crates (every package in `Cargo.lock`)

366 crates besides Wikanvas's own six (`wikanvas-api`, `wikanvas-core`, `wikanvas-store`, `wikanvas-server`,
`wikanvas-assets`, `wikanvas-cli`, all MIT). "Used for": **binaries**, compiled into `wikanvas-server` or `wikanvas` on
Linux, at run or build time (proc macros and build scripts included; 261 crates, the same set as the image's
`/licenses.txt`); **other targets**, only for Windows, macOS, WASI or wasm32 (35); **tests only**, reached only through
dev-dependencies (70). Versions and licences are as `cargo metadata --format-version 1 --locked` reports them on
2026-10-28.

| Crate | Version | Licence | Used for | Source |
|---|---|---|---|---|
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | binaries | https://github.com/oyvindln/adler2 |
| aho-corasick | 1.1.5 | Unlicense OR MIT | binaries | https://github.com/BurntSushi/aho-corasick |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 | binaries | https://github.com/zakarumych/allocator-api2 |
| android_system_properties | 0.1.6 | MIT OR Apache-2.0 | tests only | https://github.com/nical/android_system_properties |
| anstream | 1.0.0 | MIT OR Apache-2.0 | binaries | https://github.com/rust-cli/anstyle.git |
| anstyle | 1.0.14 | MIT OR Apache-2.0 | binaries | https://github.com/rust-cli/anstyle.git |
| anstyle-parse | 1.0.0 | MIT OR Apache-2.0 | binaries | https://github.com/rust-cli/anstyle.git |
| anstyle-query | 1.1.5 | MIT OR Apache-2.0 | binaries | https://github.com/rust-cli/anstyle.git |
| anstyle-wincon | 3.0.11 | MIT OR Apache-2.0 | other targets | https://github.com/rust-cli/anstyle.git |
| anyhow | 1.0.104 | MIT OR Apache-2.0 | tests only | https://github.com/dtolnay/anyhow |
| arc-swap | 1.9.2 | MIT OR Apache-2.0 | binaries | https://github.com/vorner/arc-swap |
| argon2 | 0.6.0 | MIT OR Apache-2.0 | binaries | https://github.com/RustCrypto/password-hashes |
| assert_cmd | 2.2.2 | MIT OR Apache-2.0 | tests only | https://github.com/assert-rs/assert_cmd.git |
| async-compression | 0.4.50 | MIT OR Apache-2.0 | binaries | https://github.com/Nullus157/async-compression |
| async-lock | 3.4.2 | Apache-2.0 OR MIT | binaries | https://github.com/smol-rs/async-lock |
| async-trait | 0.1.92 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/async-trait |
| atomic-waker | 1.1.2 | Apache-2.0 OR MIT | binaries | https://github.com/smol-rs/atomic-waker |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | tests only | https://github.com/cuviper/autocfg |
| axum | 0.8.9 | MIT | binaries | https://github.com/tokio-rs/axum |
| axum-core | 0.5.6 | MIT | binaries | https://github.com/tokio-rs/axum |
| axum-macros | 0.5.1 | MIT | binaries | https://github.com/tokio-rs/axum |
| axum-test | 21.1.0 | MIT | tests only | https://github.com/JosephLenton/axum-test |
| base64 | 0.23.1 | MIT OR Apache-2.0 | binaries | https://github.com/marshallpierce/rust-base64 |
| base64ct | 1.8.3 | Apache-2.0 OR MIT | binaries | https://github.com/RustCrypto/formats |
| bit-set | 0.11.1 | Apache-2.0 OR MIT | tests only | https://github.com/contain-rs/bit-vec |
| bit-vec | 0.10.1 | Apache-2.0 OR MIT | tests only | https://github.com/contain-rs/bit-vec |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | binaries | https://github.com/bitflags/bitflags |
| blake2 | 0.11.0 | MIT OR Apache-2.0 | binaries | https://github.com/RustCrypto/hashes |
| block-buffer | 0.12.1 | MIT OR Apache-2.0 | binaries | https://github.com/RustCrypto/utils |
| borsh | 1.8.1 | MIT OR Apache-2.0 | tests only | https://github.com/near/borsh-rs |
| borsh-derive | 1.8.1 | Apache-2.0 | tests only | https://github.com/near/borsh-rs |
| bstr | 1.13.1 | MIT OR Apache-2.0 | tests only | https://github.com/BurntSushi/bstr |
| bumpalo | 3.20.3 | MIT OR Apache-2.0 | other targets | https://github.com/fitzgen/bumpalo |
| bytes | 1.12.1 | MIT | binaries | https://github.com/tokio-rs/bytes |
| bytesize | 2.7.0 | Apache-2.0 | tests only | https://github.com/bytesize-rs/bytesize |
| caseless | 0.2.2 | MIT | binaries | https://github.com/unicode-rs/rust-caseless |
| cc | 1.8.0 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/cc-rs |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/cfg-if |
| cfg_aliases | 0.2.2 | MIT | tests only | https://github.com/katharostech/cfg_aliases |
| chacha20 | 0.10.2 | MIT OR Apache-2.0 | tests only | https://github.com/RustCrypto/stream-ciphers |
| chrono | 0.4.45 | MIT OR Apache-2.0 | tests only | https://github.com/chronotope/chrono |
| clap | 4.6.7 | MIT OR Apache-2.0 | binaries | https://github.com/clap-rs/clap |
| clap_builder | 4.6.7 | MIT OR Apache-2.0 | binaries | https://github.com/clap-rs/clap |
| clap_derive | 4.6.7 | MIT OR Apache-2.0 | binaries | https://github.com/clap-rs/clap |
| clap_lex | 1.1.1 | MIT OR Apache-2.0 | binaries | https://github.com/clap-rs/clap |
| cmov | 0.5.4 | Apache-2.0 OR MIT | binaries | https://github.com/RustCrypto/utils |
| colorchoice | 1.0.5 | MIT OR Apache-2.0 | binaries | https://github.com/rust-cli/anstyle.git |
| compression-codecs | 0.4.45 | MIT OR Apache-2.0 | binaries | https://github.com/Nullus157/async-compression |
| compression-core | 0.4.33 | MIT OR Apache-2.0 | binaries | https://github.com/Nullus157/async-compression |
| comrak | 0.59.0 | BSD-2-Clause | binaries | https://github.com/kivikakk/comrak |
| const-oid | 0.10.2 | Apache-2.0 OR MIT | binaries | https://github.com/RustCrypto/formats |
| convert_case | 0.10.0 | MIT | tests only | https://github.com/rutrum/convert-case |
| cookie | 0.18.2 | MIT OR Apache-2.0 | binaries | https://github.com/SergioBenitez/cookie-rs |
| cookie_store | 0.22.1 | MIT OR Apache-2.0 | binaries | https://github.com/pfernie/cookie_store |
| core-foundation-sys | 0.8.7 | MIT OR Apache-2.0 | tests only | https://github.com/servo/core-foundation-rs |
| core_detect | 1.0.0 | MIT/Apache-2.0 | tests only | https://github.com/thomcc/core_detect |
| cpufeatures | 0.3.1 | MIT OR Apache-2.0 | binaries | https://github.com/RustCrypto/utils |
| crc32fast | 1.5.2 | MIT OR Apache-2.0 | binaries | https://github.com/srijs/rust-crc32fast |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 | binaries | https://github.com/crossbeam-rs/crossbeam |
| crypto-common | 0.2.2 | MIT OR Apache-2.0 | binaries | https://github.com/RustCrypto/traits |
| ctutils | 0.4.3 | Apache-2.0 OR MIT | binaries | https://github.com/RustCrypto/utils |
| dashmap | 6.2.1 | MIT | binaries | https://github.com/xacrimon/dashmap |
| deadpool | 0.13.1 | MIT OR Apache-2.0 | binaries | https://github.com/deadpool-rs/deadpool |
| deadpool-runtime | 0.3.1 | MIT OR Apache-2.0 | binaries | https://github.com/deadpool-rs/deadpool |
| deadpool-sqlite | 0.14.0 | MIT OR Apache-2.0 | binaries | https://github.com/deadpool-rs/deadpool |
| deadpool-sync | 0.2.0 | MIT OR Apache-2.0 | binaries | https://github.com/deadpool-rs/deadpool |
| deranged | 0.5.9 | MIT OR Apache-2.0 | binaries | https://github.com/jhpratt/deranged |
| derive_more | 2.1.1 | MIT | tests only | https://github.com/JelteF/derive_more |
| derive_more-impl | 2.1.1 | MIT | tests only | https://github.com/JelteF/derive_more |
| diff | 0.1.13 | MIT OR Apache-2.0 | tests only | https://github.com/utkarshkukreti/diff.rs |
| difflib | 0.4.0 | MIT | tests only | https://github.com/DimaKudosh/difflib |
| digest | 0.11.3 | MIT OR Apache-2.0 | binaries | https://github.com/RustCrypto/traits |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 | binaries | https://github.com/yaahc/displaydoc |
| document-features | 0.2.12 | MIT OR Apache-2.0 | binaries | https://github.com/slint-ui/document-features |
| educe | 0.7.6 | MIT | tests only | https://github.com/magiclen/educe |
| email_address | 0.2.9 | MIT | tests only | https://github.com/johnstonskj/rust-email_address.git |
| entities | 1.0.1 | MIT | binaries | https://github.com/p-jackson/entities |
| enum-ordinalize | 4.4.2 | MIT | tests only | https://github.com/magiclen/enum-ordinalize |
| enum-ordinalize-derive | 4.4.2 | MIT | tests only | https://github.com/magiclen/enum-ordinalize |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | binaries | https://github.com/indexmap-rs/equivalent |
| erased-serde | 0.4.10 | MIT OR Apache-2.0 | tests only | https://github.com/dtolnay/erased-serde |
| errno | 0.3.14 | MIT OR Apache-2.0 | binaries | https://github.com/lambda-fairy/rust-errno |
| etcetera | 0.11.0 | MIT OR Apache-2.0 | binaries | https://github.com/lunacookies/etcetera |
| event-listener | 5.4.2 | Apache-2.0 OR MIT | binaries | https://github.com/smol-rs/event-listener |
| event-listener-strategy | 0.5.4 | Apache-2.0 OR MIT | binaries | https://github.com/smol-rs/event-listener-strategy |
| expect-json | 1.11.0 | MIT | tests only | https://github.com/JosephLenton/expect-json |
| expect-json-macros | 1.11.0 | MIT | tests only | https://github.com/JosephLenton/expect-json |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 | binaries | https://github.com/sfackler/rust-fallible-iterator |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 | binaries | https://github.com/sfackler/fallible-streaming-iterator |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | binaries | https://github.com/smol-rs/fastrand |
| fearless_simd | 1.1.0 | Apache-2.0 OR MIT | binaries | https://github.com/linebender/fearless_simd |
| fearless_simd_macros | 0.1.0 | Apache-2.0 OR MIT | binaries | https://github.com/linebender/fearless_simd |
| filetime | 0.2.29 | MIT/Apache-2.0 | binaries | https://github.com/alexcrichton/filetime |
| find-msvc-tools | 0.1.14 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/cc-rs |
| finl_unicode | 1.5.0 | (MIT OR Apache-2.0) AND Unicode-DFS-2016 | binaries | https://github.com/dahosek/finl_unicode |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/flate2-rs |
| fnv | 1.0.7 | Apache-2.0 / MIT | tests only | https://github.com/servo/rust-fnv |
| foldhash | 0.2.0 | Zlib | binaries | https://github.com/orlp/foldhash |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | binaries | https://github.com/servo/rust-url |
| futures-channel | 0.3.34 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/futures-rs |
| futures-core | 0.3.34 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/futures-rs |
| futures-io | 0.3.34 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/futures-rs |
| futures-macro | 0.3.34 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/futures-rs |
| futures-sink | 0.3.34 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/futures-rs |
| futures-task | 0.3.34 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/futures-rs |
| futures-timer | 3.0.4 | MIT/Apache-2.0 | binaries | https://github.com/async-rs/futures-timer |
| futures-util | 0.3.34 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/futures-rs |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | binaries | https://github.com/rust-random/getrandom |
| getrandom | 0.3.4 | MIT OR Apache-2.0 | binaries | https://github.com/rust-random/getrandom |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | binaries | https://github.com/rust-random/getrandom |
| governor | 0.10.4 | MIT | binaries | https://github.com/boinkor-net/governor.git |
| hashbrown | 0.14.5 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.16.1 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/hashbrown |
| hashlink | 0.12.2 | MIT OR Apache-2.0 | binaries | https://github.com/djc/hashlink |
| heck | 0.5.0 | MIT OR Apache-2.0 | binaries | https://github.com/withoutboats/heck |
| http | 1.5.0 | MIT OR Apache-2.0 | binaries | https://github.com/hyperium/http |
| http-body | 1.1.0 | MIT | binaries | https://github.com/hyperium/http-body |
| http-body-util | 0.1.5 | MIT | binaries | https://github.com/hyperium/http-body |
| httparse | 1.10.1 | MIT OR Apache-2.0 | binaries | https://github.com/seanmonstar/httparse |
| httpdate | 1.0.3 | MIT OR Apache-2.0 | binaries | https://github.com/pyfisch/httpdate |
| hybrid-array | 0.4.15 | MIT OR Apache-2.0 | binaries | https://github.com/RustCrypto/hybrid-array |
| hyper | 1.12.0 | MIT | binaries | https://github.com/hyperium/hyper |
| hyper-util | 0.1.21 | MIT | binaries | https://github.com/hyperium/hyper-util |
| iana-time-zone | 0.1.65 | MIT OR Apache-2.0 | tests only | https://github.com/strawlab/iana-time-zone |
| iana-time-zone-haiku | 0.1.2 | MIT OR Apache-2.0 | tests only | https://github.com/strawlab/iana-time-zone |
| icu_collections | 2.3.0 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| icu_locale_core | 2.3.0 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| icu_normalizer | 2.3.0 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| icu_normalizer_data | 2.3.0 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| icu_properties | 2.3.0 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| icu_properties_data | 2.3.0 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| icu_provider | 2.3.1 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| idna | 1.1.0 | MIT OR Apache-2.0 | binaries | https://github.com/servo/rust-url/ |
| idna_adapter | 1.2.2 | Apache-2.0 OR MIT | binaries | https://github.com/hsivonen/idna_adapter |
| imagesize | 0.15.0 | MIT | binaries | https://github.com/Roughsketch/imagesize |
| include_dir | 0.7.4 | MIT | binaries | https://github.com/Michael-F-Bryan/include_dir |
| include_dir_macros | 0.7.4 | MIT | binaries | https://github.com/Michael-F-Bryan/include_dir |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | binaries | https://github.com/indexmap-rs/indexmap |
| inkboard-core | 0.2.0-beta.3 | MIT OR Apache-2.0 | binaries | vendor/inkboard (submodule) |
| inventory | 0.3.25 | MIT OR Apache-2.0 | tests only | https://github.com/dtolnay/inventory |
| is_terminal_polyfill | 1.70.2 | MIT OR Apache-2.0 | binaries | https://github.com/polyfill-rs/is_terminal_polyfill |
| itoa | 1.0.18 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/itoa |
| js-sys | 0.3.106 | MIT OR Apache-2.0 | other targets | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/js-sys |
| lazy_static | 1.5.1 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang-nursery/lazy-static.rs |
| libc | 0.2.190 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/libc |
| libsqlite3-sys | 0.38.2 | MIT | binaries | https://github.com/rusqlite/rusqlite |
| linux-raw-sys | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | binaries | https://github.com/sunfishcode/linux-raw-sys |
| litemap | 0.8.3 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| litrs | 1.0.1 | MIT OR Apache-2.0 | binaries | https://github.com/LukasKalbertodt/litrs |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | binaries | https://github.com/Amanieu/parking_lot |
| log | 0.4.34 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/log |
| matchers | 0.2.0 | MIT | binaries | https://github.com/hawkw/matchers |
| matchit | 0.8.4 | MIT AND BSD-3-Clause | binaries | https://github.com/ibraheemdev/matchit |
| memchr | 2.8.3 | Unlicense OR MIT | binaries | https://github.com/BurntSushi/memchr |
| memchr-n | 0.1.9 | MIT OR Apache-2.0 | binaries | https://github.com/Dr-Emann/memchr_n |
| mime | 0.3.17 | MIT OR Apache-2.0 | binaries | https://github.com/hyperium/mime |
| mime_guess | 2.0.5 | MIT | binaries | https://github.com/abonander/mime_guess |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | binaries | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| mio | 1.2.4 | MIT | binaries | https://github.com/tokio-rs/mio |
| nonzero_ext | 0.3.0 | Apache-2.0 | binaries | https://github.com/antifuchs/nonzero_ext |
| nu-ansi-term | 0.50.3 | MIT | binaries | https://github.com/nushell/nu-ansi-term |
| num | 0.4.3 | MIT OR Apache-2.0 | tests only | https://github.com/rust-num/num |
| num-bigint | 0.4.8 | MIT OR Apache-2.0 | tests only | https://github.com/rust-num/num-bigint |
| num-complex | 0.4.6 | MIT OR Apache-2.0 | tests only | https://github.com/rust-num/num-complex |
| num-conv | 0.2.2 | MIT OR Apache-2.0 | binaries | https://github.com/jhpratt/num-conv |
| num-integer | 0.1.47 | MIT OR Apache-2.0 | tests only | https://github.com/rust-num/num-integer |
| num-iter | 0.1.46 | MIT OR Apache-2.0 | tests only | https://github.com/rust-num/num-iter |
| num-rational | 0.4.2 | MIT OR Apache-2.0 | tests only | https://github.com/rust-num/num-rational |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | tests only | https://github.com/rust-num/num-traits |
| oas3 | 0.22.0 | MIT | tests only | https://github.com/x52dev/oas3-rs |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | binaries | https://github.com/matklad/once_cell |
| once_cell_polyfill | 1.70.2 | MIT OR Apache-2.0 | other targets | https://github.com/polyfill-rs/once_cell_polyfill |
| parking | 2.2.1 | Apache-2.0 OR MIT | binaries | https://github.com/smol-rs/parking |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | binaries | https://github.com/Amanieu/parking_lot |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 | binaries | https://github.com/Amanieu/parking_lot |
| password-hash | 0.6.1 | MIT OR Apache-2.0 | binaries | https://github.com/RustCrypto/traits |
| pastey | 0.2.3 | MIT OR Apache-2.0 | binaries | https://github.com/as1100k/pastey |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | binaries | https://github.com/servo/rust-url/ |
| phc | 0.6.1 | Apache-2.0 OR MIT | binaries | https://github.com/RustCrypto/formats |
| phf | 0.14.0 | MIT | binaries | https://github.com/rust-phf/rust-phf |
| phf_codegen | 0.14.0 | MIT | binaries | https://github.com/rust-phf/rust-phf |
| phf_generator | 0.14.0 | MIT | binaries | https://github.com/rust-phf/rust-phf |
| phf_shared | 0.14.0 | MIT | binaries | https://github.com/rust-phf/rust-phf |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | binaries | https://github.com/taiki-e/pin-project-lite |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/pkg-config-rs |
| portable-atomic | 1.15.0 | Apache-2.0 OR MIT | binaries | https://github.com/taiki-e/portable-atomic |
| potential_utf | 0.1.6 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| powerfmt | 0.2.1 | MIT OR Apache-2.0 | binaries | https://github.com/jhpratt/powerfmt |
| ppv-lite86 | 0.2.21 | MIT OR Apache-2.0 | binaries | https://github.com/cryptocorrosion/cryptocorrosion |
| predicates | 3.1.4 | MIT OR Apache-2.0 | tests only | https://github.com/assert-rs/predicates-rs |
| predicates-core | 1.0.10 | MIT OR Apache-2.0 | tests only | https://github.com/assert-rs/predicates-rs |
| predicates-tree | 1.0.13 | MIT OR Apache-2.0 | tests only | https://github.com/assert-rs/predicates-rs |
| pretty_assertions | 1.4.1 | MIT OR Apache-2.0 | tests only | https://github.com/rust-pretty-assertions/rust-pretty-assertions |
| proc-macro-crate | 3.5.0 | MIT OR Apache-2.0 | tests only | https://github.com/bkchr/proc-macro-crate |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/proc-macro2 |
| proptest | 1.12.0 | MIT OR Apache-2.0 | tests only | https://github.com/proptest-rs/proptest |
| quanta | 0.12.6 | MIT | binaries | https://github.com/metrics-rs/quanta |
| quick-error | 1.2.3 | MIT/Apache-2.0 | tests only | http://github.com/tailhook/quick-error |
| quote | 1.0.47 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/quote |
| r-efi | 5.3.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | other targets | https://github.com/r-efi/r-efi |
| r-efi | 6.0.0 | MIT OR Apache-2.0 OR LGPL-2.1-or-later | other targets | https://github.com/r-efi/r-efi |
| rand | 0.9.5 | MIT OR Apache-2.0 | binaries | https://github.com/rust-random/rand |
| rand | 0.10.3 | MIT OR Apache-2.0 | tests only | https://github.com/rust-random/rand |
| rand_chacha | 0.9.0 | MIT OR Apache-2.0 | binaries | https://github.com/rust-random/rand |
| rand_core | 0.9.5 | MIT OR Apache-2.0 | binaries | https://github.com/rust-random/rand |
| rand_core | 0.10.1 | MIT OR Apache-2.0 | binaries | https://github.com/rust-random/rand_core |
| rand_xorshift | 0.5.0 | MIT OR Apache-2.0 | tests only | https://github.com/rust-random/rngs |
| raw-cpuid | 11.6.0 | MIT | binaries | https://github.com/gz/rust-cpuid |
| redox_syscall | 0.5.18 | MIT | other targets | https://gitlab.redox-os.org/redox-os/syscall |
| regex | 1.13.1 | MIT OR Apache-2.0 | tests only | https://github.com/rust-lang/regex |
| regex-automata | 0.4.18 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/regex |
| regex-syntax | 0.8.11 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/regex |
| reserve-port | 2.5.0 | MIT | tests only | https://github.com/JosephLenton/reserve-port |
| ring | 0.17.14 | Apache-2.0 AND ISC | binaries | https://github.com/briansmith/ring |
| rsqlite-vfs | 0.1.1 | MIT | other targets | crates.io |
| rusqlite | 0.40.2 | MIT | binaries | https://github.com/rusqlite/rusqlite |
| rusqlite_migration | 2.6.0 | Apache-2.0 | binaries | https://github.com/cljoly/rusqlite_migration |
| rust-embed | 8.13.0 | MIT | binaries | https://pyrossh.dev/repos/rust-embed |
| rust-embed-impl | 8.13.0 | MIT | binaries | https://pyrossh.dev/repos/rust-embed |
| rust-embed-utils | 8.13.0 | MIT | binaries | https://pyrossh.dev/repos/rust-embed |
| rust-multipart-rfc7578_2 | 0.9.0 | MIT OR Apache-2.0 | tests only | https://github.com/JosephLenton/rust-multipart-rfc7578_2 |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT | binaries | https://github.com/rust-lang/rustc-hash |
| rustc_version | 0.4.1 | MIT OR Apache-2.0 | tests only | https://github.com/djc/rustc-version-rs |
| rustix | 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | binaries | https://github.com/bytecodealliance/rustix |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT | binaries | https://github.com/rustls/rustls |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 | binaries | https://github.com/rustls/pki-types |
| rustls-webpki | 0.103.15 | ISC | binaries | https://github.com/rustls/webpki |
| rustversion | 1.0.23 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/rustversion |
| rusty-fork | 0.3.1 | MIT/Apache-2.0 | tests only | https://github.com/altsysrq/rusty-fork |
| ryu | 1.0.23 | Apache-2.0 OR BSL-1.0 | binaries | https://github.com/dtolnay/ryu |
| same-file | 1.0.6 | Unlicense/MIT | binaries | https://github.com/BurntSushi/same-file |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 | binaries | https://github.com/bluss/scopeguard |
| semver | 1.0.28 | MIT OR Apache-2.0 | tests only | https://github.com/dtolnay/semver |
| serde | 1.0.229 | MIT OR Apache-2.0 | binaries | https://github.com/serde-rs/serde |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | binaries | https://github.com/serde-rs/serde |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | binaries | https://github.com/serde-rs/serde |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | binaries | https://github.com/serde-rs/json |
| serde_path_to_error | 0.1.20 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/path-to-error |
| serde_spanned | 1.1.2 | MIT OR Apache-2.0 | binaries | https://github.com/toml-rs/toml |
| serde_urlencoded | 0.7.1 | MIT/Apache-2.0 | binaries | https://github.com/nox/serde_urlencoded |
| sha2 | 0.11.1 | MIT OR Apache-2.0 | binaries | https://github.com/RustCrypto/hashes |
| sharded-slab | 0.1.7 | MIT | binaries | https://github.com/hawkw/sharded-slab |
| shlex | 2.0.1 | MIT OR Apache-2.0 | binaries | https://github.com/comex/rust-shlex |
| signal-hook-registry | 1.4.8 | MIT OR Apache-2.0 | binaries | https://github.com/vorner/signal-hook |
| simd-adler32 | 0.3.10 | MIT | binaries | https://github.com/mcountryman/simd-adler32 |
| siphasher | 1.0.4 | MIT OR Apache-2.0 | binaries | https://github.com/jedisct1/rust-siphash |
| slab | 0.4.12 | MIT | binaries | https://github.com/tokio-rs/slab |
| smallstr | 0.3.1 | MIT OR Apache-2.0 | binaries | https://github.com/murarth/smallstr |
| smallvec | 1.16.3 | MIT OR Apache-2.0 | binaries | https://github.com/servo/rust-smallvec |
| socket2 | 0.6.5 | MIT OR Apache-2.0 | binaries | https://github.com/rust-lang/socket2 |
| spinning_top | 0.3.0 | MIT/Apache-2.0 | binaries | https://github.com/rust-osdev/spinning_top |
| sqlite-wasm-rs | 0.5.5 | MIT | other targets | https://github.com/Spxg/sqlite-wasm-rs |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | binaries | https://github.com/storyyeller/stable_deref_trait |
| strsim | 0.11.1 | MIT | binaries | https://github.com/rapidfuzz/strsim-rs |
| subtle | 2.6.1 | BSD-3-Clause | binaries | https://github.com/dalek-cryptography/subtle |
| syn | 2.0.119 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/syn |
| syn | 3.0.9 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/syn |
| sync_wrapper | 1.0.2 | Apache-2.0 | binaries | https://github.com/Actyx/sync_wrapper |
| synstructure | 0.14.0 | MIT | binaries | https://github.com/mystor/synstructure |
| tar | 0.4.46 | MIT OR Apache-2.0 | binaries | https://github.com/composefs/tar-rs |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | binaries | https://github.com/Stebalien/tempfile |
| termcolor | 1.4.1 | Unlicense OR MIT | binaries | https://github.com/BurntSushi/termcolor |
| termtree | 0.5.1 | MIT | tests only | https://github.com/rust-cli/termtree |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/thiserror |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | binaries | https://github.com/dtolnay/thiserror |
| thread_local | 1.1.10 | MIT OR Apache-2.0 | binaries | https://github.com/Amanieu/thread_local-rs |
| time | 0.3.55 | MIT OR Apache-2.0 | binaries | https://github.com/time-rs/time |
| time-core | 0.1.9 | MIT OR Apache-2.0 | binaries | https://github.com/time-rs/time |
| time-macros | 0.2.32 | MIT OR Apache-2.0 | binaries | https://github.com/time-rs/time |
| tinystr | 0.8.4 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| tinyvec | 1.13.3 | Zlib OR Apache-2.0 OR MIT | binaries | https://github.com/Lokathor/tinyvec |
| tokio | 1.54.1 | MIT | binaries | https://github.com/tokio-rs/tokio |
| tokio-macros | 2.7.2 | MIT | binaries | https://github.com/tokio-rs/tokio |
| tokio-util | 0.7.20 | MIT | binaries | https://github.com/tokio-rs/tokio |
| toml | 1.1.8+spec-1.1.0 | MIT OR Apache-2.0 | binaries | https://github.com/toml-rs/toml |
| toml_datetime | 1.1.2+spec-1.1.0 | MIT OR Apache-2.0 | binaries | https://github.com/toml-rs/toml |
| toml_edit | 0.25.17+spec-1.1.0 | MIT OR Apache-2.0 | tests only | https://github.com/toml-rs/toml |
| toml_parser | 1.1.5+spec-1.1.0 | MIT OR Apache-2.0 | binaries | https://github.com/toml-rs/toml |
| toml_writer | 1.1.3+spec-1.1.0 | MIT OR Apache-2.0 | binaries | https://github.com/toml-rs/toml |
| tower | 0.5.3 | MIT | binaries | https://github.com/tower-rs/tower |
| tower-http | 0.7.1 | MIT | binaries | https://github.com/tower-rs/tower-http |
| tower-layer | 0.3.3 | MIT | binaries | https://github.com/tower-rs/tower |
| tower-service | 0.3.3 | MIT | binaries | https://github.com/tower-rs/tower |
| tracing | 0.1.44 | MIT | binaries | https://github.com/tokio-rs/tracing |
| tracing-attributes | 0.1.31 | MIT | binaries | https://github.com/tokio-rs/tracing |
| tracing-core | 0.1.36 | MIT | binaries | https://github.com/tokio-rs/tracing |
| tracing-log | 0.2.0 | MIT | binaries | https://github.com/tokio-rs/tracing |
| tracing-subscriber | 0.3.23 | MIT | binaries | https://github.com/tokio-rs/tracing |
| try-lock | 0.2.5 | MIT | binaries | https://github.com/seanmonstar/try-lock |
| ts-rs | 12.0.1 | MIT | binaries | https://github.com/Aleph-Alpha/ts-rs |
| ts-rs-macros | 12.0.1 | MIT | binaries | https://github.com/Aleph-Alpha/ts-rs |
| typed-arena | 2.0.2 | MIT | binaries | https://github.com/SimonSapin/rust-typed-arena |
| typeid | 1.0.3 | MIT OR Apache-2.0 | tests only | https://github.com/dtolnay/typeid |
| typenum | 1.20.1 | MIT OR Apache-2.0 | binaries | https://github.com/paholg/typenum |
| typetag | 0.2.23 | MIT OR Apache-2.0 | tests only | https://github.com/dtolnay/typetag |
| typetag-impl | 0.2.23 | MIT OR Apache-2.0 | tests only | https://github.com/dtolnay/typetag |
| unarray | 0.1.4 | MIT OR Apache-2.0 | tests only | https://github.com/cameron1024/unarray |
| unicase | 2.10.0 | MIT OR Apache-2.0 | binaries | https://github.com/seanmonstar/unicase |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | binaries | https://github.com/dtolnay/unicode-ident |
| unicode-normalization | 0.1.25 | MIT OR Apache-2.0 | binaries | https://github.com/unicode-rs/unicode-normalization |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 | tests only | https://github.com/unicode-rs/unicode-segmentation |
| unicode-xid | 0.2.6 | MIT OR Apache-2.0 | tests only | https://github.com/unicode-rs/unicode-xid |
| untrusted | 0.9.0 | ISC | binaries | https://github.com/briansmith/untrusted |
| ureq | 3.5.0 | MIT OR Apache-2.0 | binaries | https://github.com/algesten/ureq |
| ureq-proto | 0.6.5 | MIT OR Apache-2.0 | binaries | https://github.com/algesten/ureq-proto |
| url | 2.5.8 | MIT OR Apache-2.0 | binaries | https://github.com/servo/rust-url |
| utf8-zero | 0.8.1 | MIT OR Apache-2.0 | binaries | https://github.com/algesten/utf8-zero |
| utf8_iter | 1.0.4 | Apache-2.0 OR MIT | binaries | https://github.com/hsivonen/utf8_iter |
| utf8parse | 0.2.2 | Apache-2.0 OR MIT | binaries | https://github.com/alacritty/vte |
| utoipa | 6.0.0 | MIT OR Apache-2.0 | binaries | https://github.com/juhaku/utoipa |
| utoipa-axum | 0.3.0 | MIT OR Apache-2.0 | binaries | https://github.com/juhaku/utoipa |
| utoipa-gen | 6.0.1 | MIT OR Apache-2.0 | binaries | https://github.com/juhaku/utoipa |
| uuid | 1.29.0 | Apache-2.0 OR MIT | binaries | https://github.com/uuid-rs/uuid |
| valuable | 0.1.1 | MIT | other targets | https://github.com/tokio-rs/valuable |
| vcpkg | 0.2.15 | MIT/Apache-2.0 | binaries | https://github.com/mcgoo/vcpkg-rs |
| version_check | 0.9.5 | MIT/Apache-2.0 | binaries | https://github.com/SergioBenitez/version_check |
| wait-timeout | 0.2.1 | MIT/Apache-2.0 | tests only | https://github.com/alexcrichton/wait-timeout |
| walkdir | 2.5.0 | Unlicense/MIT | binaries | https://github.com/BurntSushi/walkdir |
| want | 0.3.2 | MIT | binaries | https://github.com/seanmonstar/want |
| wasi | 0.11.1+wasi-snapshot-preview1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | other targets | https://github.com/bytecodealliance/wasi |
| wasip2 | 1.0.4+wasi-0.2.12 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | other targets | https://github.com/bytecodealliance/wasi-rs |
| wasm-bindgen | 0.2.129 | MIT OR Apache-2.0 | other targets | https://github.com/wasm-bindgen/wasm-bindgen |
| wasm-bindgen-macro | 0.2.129 | MIT OR Apache-2.0 | other targets | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/macro |
| wasm-bindgen-macro-support | 0.2.129 | MIT OR Apache-2.0 | other targets | https://github.com/wasm-bindgen/wasm-bindgen/tree/main/crates/macro-support |
| wasm-bindgen-shared | 0.2.129 | MIT OR Apache-2.0 | other targets | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/shared |
| web-sys | 0.3.106 | MIT OR Apache-2.0 | other targets | https://github.com/wasm-bindgen/wasm-bindgen/tree/master/crates/web-sys |
| web-time | 1.1.0 | MIT OR Apache-2.0 | binaries | https://github.com/daxpedda/web-time |
| winapi | 0.3.9 | MIT/Apache-2.0 | other targets | https://github.com/retep998/winapi-rs |
| winapi-i686-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 | other targets | https://github.com/retep998/winapi-rs |
| winapi-util | 0.1.11 | Unlicense OR MIT | other targets | https://github.com/BurntSushi/winapi-util |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 | other targets | https://github.com/retep998/winapi-rs |
| windows-core | 0.62.2 | MIT OR Apache-2.0 | tests only | https://github.com/microsoft/windows-rs |
| windows-implement | 0.60.2 | MIT OR Apache-2.0 | tests only | https://github.com/microsoft/windows-rs |
| windows-interface | 0.59.3 | MIT OR Apache-2.0 | tests only | https://github.com/microsoft/windows-rs |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows-result | 0.4.1 | MIT OR Apache-2.0 | tests only | https://github.com/microsoft/windows-rs |
| windows-strings | 0.5.1 | MIT OR Apache-2.0 | tests only | https://github.com/microsoft/windows-rs |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 | other targets | https://github.com/microsoft/windows-rs |
| winnow | 1.0.4 | MIT | binaries | https://github.com/winnow-rs/winnow |
| wit-bindgen | 0.57.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | other targets | https://github.com/bytecodealliance/wit-bindgen |
| writeable | 0.6.4 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| yansi | 1.0.1 | MIT OR Apache-2.0 | tests only | https://github.com/SergioBenitez/yansi |
| yoke | 0.8.3 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| yoke-derive | 0.8.4 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| yrs | 0.28.0 | MIT | binaries | https://github.com/y-crdt/y-crdt/ |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | binaries | https://github.com/google/zerocopy |
| zerocopy-derive | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | other targets | https://github.com/google/zerocopy |
| zerofrom | 0.1.8 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| zerofrom-derive | 0.1.8 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| zeroize | 1.9.1 | Apache-2.0 OR MIT | binaries | https://github.com/RustCrypto/utils |
| zerotrie | 0.2.5 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| zerovec | 0.11.8 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| zerovec-derive | 0.11.6 | Unicode-3.0 | binaries | https://github.com/unicode-org/icu4x |
| zlib-rs | 0.6.8 | Zlib | binaries | https://github.com/trifectatechfoundation/zlib-rs |
| zmij | 1.0.23 | MIT | binaries | https://github.com/dtolnay/zmij |

## npm packages bundled into `web/dist`

The production dependencies of `web/package.json` and everything they pull in: 102 packages, as
`pnpm --dir web licenses list --json --prod` reports them on 2026-10-28 (96 MIT, 4 Apache-2.0, 1 BSD-3-Clause, 1 ISC).
Vite bundles the parts of them the client imports; the list is the whole set, so it may name a package of which
nothing ends up in the bundle (for example `@atlaskit/pragmatic-drag-and-drop-hitbox`, declared but unused).

| Package | Version | Licence | Source |
|---|---|---|---|
| @atlaskit/pragmatic-drag-and-drop | 4.0.0 | Apache-2.0 | https://atlassian.design/components/pragmatic-drag-and-drop/ |
| @atlaskit/pragmatic-drag-and-drop-auto-scroll | 3.2.1 | Apache-2.0 | https://atlassian.design/components/pragmatic-drag-and-drop/ |
| @atlaskit/pragmatic-drag-and-drop-hitbox | 3.0.0 | Apache-2.0 | https://atlassian.design/components/pragmatic-drag-and-drop/ |
| @babel/runtime | 7.29.10 | MIT | https://babel.dev/docs/en/next/babel-runtime |
| @floating-ui/core | 1.8.0 | MIT | https://floating-ui.com |
| @floating-ui/dom | 1.8.0 | MIT | https://floating-ui.com |
| @floating-ui/utils | 0.2.12 | MIT | https://floating-ui.com |
| @remix-run/route-pattern | 0.22.1 | MIT | https://github.com/remix-run/remix/tree/main/packages/route-pattern#readme |
| @tanstack/query-core | 5.104.2 | MIT | https://tanstack.com/query |
| @tanstack/react-query | 5.104.2 | MIT | https://tanstack.com/query |
| @tanstack/react-virtual | 3.14.19 | MIT | https://tanstack.com/virtual |
| @tanstack/virtual-core | 3.19.4 | MIT | https://tanstack.com/virtual |
| @tiptap/core | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-blockquote | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-bold | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-bubble-menu | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-bullet-list | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-code | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-code-block | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-code-block-lowlight | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-collaboration | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-document | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-drag-handle | 3.32.0 | MIT | https://tiptap.dev/docs/editor/extensions/functionality/drag-handle |
| @tiptap/extension-drag-handle-react | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-dropcursor | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-file-handler | 3.32.0 | MIT | https://tiptap.dev/docs/editor/extensions/functionality/filehandler |
| @tiptap/extension-floating-menu | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-gapcursor | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-hard-break | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-heading | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-highlight | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-horizontal-rule | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-image | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-italic | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-link | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-list | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-list-item | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-list-keymap | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-mention | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-node-range | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-ordered-list | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-paragraph | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-placeholder | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-strike | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-table | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-task-item | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-task-list | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-text | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-text-style | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-typography | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-underline | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/extension-unique-id | 3.32.0 | MIT | https://tiptap.dev/api/extensions/unique-id |
| @tiptap/extensions | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/pm | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/react | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/starter-kit | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/suggestion | 3.32.0 | MIT | https://tiptap.dev |
| @tiptap/y-tiptap | 3.0.10 | MIT | https://tiptap.dev |
| @types/hast | 3.0.5 | MIT | https://github.com/DefinitelyTyped/DefinitelyTyped/tree/master/types/hast |
| @types/react | 19.3.0 | MIT | https://github.com/DefinitelyTyped/DefinitelyTyped/tree/master/types/react |
| @types/react-dom | 19.3.0 | MIT | https://github.com/DefinitelyTyped/DefinitelyTyped/tree/master/types/react-dom |
| @types/unist | 3.0.3 | MIT | https://github.com/DefinitelyTyped/DefinitelyTyped/tree/master/types/unist |
| @types/use-sync-external-store | 0.0.6 | MIT | https://github.com/DefinitelyTyped/DefinitelyTyped/tree/master/types/use-sync-external-store |
| bind-event-listener | 3.0.0 | MIT | https://github.com/alexreardon/bind-event-listener#readme |
| cookie-es | 3.1.1 | MIT | https://github.com/unjs/cookie-es#readme |
| csstype | 3.2.3 | MIT | https://github.com/frenic/csstype#readme |
| dequal | 2.0.3 | MIT | https://github.com/lukeed/dequal#readme |
| devlop | 1.1.0 | MIT | https://github.com/wooorm/devlop#readme |
| fast-equals | 5.4.4 | MIT | https://github.com/planttheidea/fast-equals#readme |
| highlight.js | 11.11.2 | BSD-3-Clause | https://highlightjs.org/ |
| idb-keyval | 6.3.0 | Apache-2.0 | https://github.com/jakearchibald/idb-keyval#readme |
| isomorphic.js | 0.2.5 | MIT | https://github.com/dmonad/isomorphic.js#readme |
| lib0 | 0.2.119 | MIT | https://github.com/dmonad/lib0#readme |
| linkifyjs | 4.3.3 | MIT | https://linkify.js.org |
| lowlight | 3.3.0 | MIT | https://github.com/wooorm/lowlight#readme |
| lucide-react | 1.56.0 | ISC | https://lucide.dev |
| orderedmap | 2.1.1 | MIT | https://github.com/marijnh/orderedmap#readme |
| prosemirror-changeset | 2.4.4 | MIT | https://code.haverbeke.berlin/prosemirror/prosemirror-changeset |
| prosemirror-commands | 1.7.2 | MIT | https://code.haverbeke.berlin/prosemirror/prosemirror-commands |
| prosemirror-dropcursor | 1.8.4 | MIT | https://code.haverbeke.berlin/prosemirror/prosemirror-dropcursor |
| prosemirror-gapcursor | 1.4.1 | MIT | https://github.com/prosemirror/prosemirror-gapcursor#readme |
| prosemirror-history | 1.5.1 | MIT | https://code.haverbeke.berlin/prosemirror/prosemirror-history |
| prosemirror-inputrules | 1.5.1 | MIT | https://github.com/prosemirror/prosemirror-inputrules#readme |
| prosemirror-keymap | 1.2.3 | MIT | https://github.com/prosemirror/prosemirror-keymap#readme |
| prosemirror-model | 1.25.12 | MIT | https://code.haverbeke.berlin/prosemirror/prosemirror-model |
| prosemirror-schema-list | 1.5.1 | MIT | https://github.com/prosemirror/prosemirror-schema-list#readme |
| prosemirror-state | 1.4.4 | MIT | https://github.com/prosemirror/prosemirror-state#readme |
| prosemirror-tables | 1.8.5 | MIT | https://github.com/ProseMirror/prosemirror-tables#readme |
| prosemirror-transform | 1.12.2 | MIT | https://code.haverbeke.berlin/prosemirror/prosemirror-transform |
| prosemirror-view | 1.42.6 | MIT | https://code.haverbeke.berlin/prosemirror/prosemirror-view |
| raf-schd | 4.0.3 | MIT | https://github.com/alexreardon/raf-schd#readme |
| react | 19.3.1 | MIT | https://react.dev/ |
| react-dom | 19.3.1 | MIT | https://react.dev/ |
| react-router | 8.4.0 | MIT | https://github.com/remix-run/react-router#readme |
| rope-sequence | 1.3.4 | MIT | https://github.com/marijnh/rope-sequence#readme |
| scheduler | 0.28.0 | MIT | https://react.dev/ |
| use-sync-external-store | 1.7.0 | MIT | https://github.com/react/react#readme |
| uuid | 14.0.3 | MIT | https://github.com/uuidjs/uuid#readme |
| w3c-keyname | 2.2.8 | MIT | https://github.com/marijnh/w3c-keyname#readme |
| y-protocols | 1.0.7 | MIT | https://github.com/yjs/y-protocols#readme |
| yjs | 13.6.33 | MIT | https://docs.yjs.dev |
| zustand | 5.0.15 | MIT | https://github.com/pmndrs/zustand |

## Build and test tools (not distributed)

- **The web client's devDependencies** and what they pull in (62 more packages): vite 8.3.5, vitest 5.0.5, typescript
  7.0.2, happy-dom, @testing-library/*, fake-indexeddb, @vitejs/plugin-react, rolldown, postcss and others. 51 are MIT,
  6 Apache-2.0, 1 BSD-2-Clause (entities), 1 BSD-3-Clause (source-map-js), 1 ISC (picocolors), and 2 MPL-2.0
  (lightningcss and lightningcss-linux-x64-gnu, see above). `pnpm --dir web licenses list --json` lists them.
- **The e2e package** `e2e/`: @playwright/test 1.64.1 with playwright and playwright-core (Apache-2.0), run from the
  `mcr.microsoft.com/playwright:v1.64.1-noble` image.
- **The toolchain image** (`docker/Dockerfile`, `toolchain-base` and `toolchain`): Rust 1.99.1, Node 24, pnpm 11.24.0,
  binaryen 131, wasm-bindgen-cli 0.2.129, trunk 0.22.1, clang/lld/llvm, brotli and musl-tools. They build Wikanvas and
  are never copied into the runtime image.

## Updating this file

When `Cargo.lock` or `web/pnpm-lock.yaml` changes, regenerate the two package tables (through the capped wrappers,
never on the host):

```sh
scripts/dev.sh cargo metadata --format-version 1 --locked > .cache/cargo-metadata.json
scripts/dev.sh pnpm --dir web licenses list --json --prod > .cache/npm-prod.json
```

A Rust row is `| name | version | license | used for | repository |` for every package with a `source` (plus
inkboard-core), sorted by name and version; "used for" comes from walking `resolve.nodes` from the workspace members
over edges that are not dev-only, once over the whole metadata and once over `--filter-platform
x86_64-unknown-linux-gnu`. An npm row is `| name | version | license | homepage |`. Check every new licence against
R6, and list an exception above. The image's `/licenses.txt` needs no update: the build regenerates it.
