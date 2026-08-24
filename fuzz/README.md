# cargo-fuzz targets (main spec §50 test 41)

Fuzz harnesses for the record-framed/body parser surfaces of
`crates/support`. This crate is deliberately **not** a workspace member:
it lives in its own `[workspace]`, so root `cargo fmt/clippy/test` are
unaffected.

## Run

```sh
cargo install cargo-fuzz --locked   # needs a nightly toolchain
cargo +nightly fuzz run parse_content_type  -- -max_total_time=20 -rss_limit_mb=2560
```

Targets: `parse_content_type`, `form_pairs`, `params_decode_query`,
`sse_decode`, `ndjson_decode`, `jsonseq_decode`, `multipart_framing`,
`pattern_match_redos`.

CI runs a **smoke pass only** (`-max_total_time=10` per target); full
campaigns are out of scope.

## Invariants asserted by every target

- **Rejection without panic**: malformed input yields the decoder's declared
  error enum (compile-time-exhaustive match per target) — never an abort.
- **Bounded memory**: items/parts are counted then dropped, consumption stops
  at 4096 items, and harness input is truncated to ≤ 32 KiB regardless of
  libFuzzer flags (`-rss_limit_mb` is the outer backstop).
- **Bounded work**: a poll hang-guard caps every driven stream; after a
  terminal error the next poll must yield `None`.
- Streaming targets feed each body as ONE chunk and as deterministic 1–7-byte
  chunks (split sizes derived from the input bytes), at generous (4096) and
  tight (64) record limits.

## Seed corpus provenance

All seeds derive from `crates/support/src/*.rs` unit-test fixtures (copied
literals).

| Target | Seeds | Provenance |
| --- | --- | --- |
| `parse_content_type` | 8 | `mediatype.rs` tests: canonical/param/case/quoted-boundary/escaped values plus §28.1 malformed rejections (`charset=` empty, unterminated quote) and a `*/*` wildcard. |
| `form_pairs` | 7 | `form.rs` tests: login flat struct, repeated-key sequence with `+`/percent UTF-8 (`caf%C3%A9`), bare token, `&&` malformed segment, `%FF` decoded-non-UTF-8, truncated `%4` escape. |
| `params_decode_query` | 6 | `params.rs` companion §6 form-style fixtures: basic pairs, exploded repeats, `+`-as-space, reserved escapes, empty value, truncated escape. |
| `sse_decode` | 6 | `sse.rs` tests: canonical comment/metadata/multi-line-data event stream, BOM body, CRLF/bare-CR terminators, `{oops}` malformed JSON, EOF-mid-event truncation, 80-byte comment line (tight-limit rejection). |
| `ndjson_decode` | 6 | `ndjson.rs` tests: canonical widget records, array records, interior blank line, EOF mid-record, non-UTF-8 line, record pre-split for chunk-edge coverage. |
| `jsonseq_decode` | 6 | `jsonseq.rs` tests (RS = `0x1E`): canonical metric records, micro scalars, junk before first RS, empty `RS LF` record, missing-LF truncation, non-UTF-8 record. |
| `multipart_framing` | 8 | `multipart.rs` tests; first byte selects the boundary (`0x00` → `XyZzy123`). Minimal part, canonical two-part body whose payload embeds boundary-shaped non-delimiter runs, empty part, LF-only framing, mid-payload truncation, preamble/epilogue, UTF-8 filename + binary payload, wrong disposition type. |
| `pattern_match_redos` | 7 | Classic catastrophic-backtracking shapes vs the bounded matcher (`(a+)+$`, `(a|aa)+$`, `(a*)*b`, `.*.*.*x`) plus simple classes, an unsupported backreference (`\1` → lenient skip), and a bounded repeat. Layout: byte 0 = pattern length ≤ 48, then pattern bytes, then subject (≤ 4 KiB). |
