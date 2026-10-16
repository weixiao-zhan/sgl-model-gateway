# slim fork of the SGL Model Gateway

Source of the wheel `sglang_router==0.3.2.post2` that slim installs. The version tracks the
upstream gateway version, and the `.post<n>` counter is this fork's patch revision.

## Provenance

| Component | Upstream | Pinned at |
| --- | --- | --- |
| Gateway (repo root) | `sgl-project/sglang`, directory `sgl-model-gateway/` | tag `v0.5.15.post1`, commit `0b3bb0cbe31873994c9f989fddfe2f87ca839fdd` |
| `crates/openai-protocol` | crates.io `openai-protocol` (Apache-2.0) | `1.0.0`, packaged from `lightseekorg/smg` commit `7455eaee1da5212ae5e0fd33d80a903029d2f981`, path `protocols` |

The gateway resolves its request and response schemas through `pub use openai_protocol as protocols`
in `src/lib.rs`, so the schema changes live in the vendored crate. The workspace `[patch.crates-io]`
entry in `Cargo.toml` redirects every dependency on `openai-protocol` — including the transitive ones
inside the published `smg-*` crates — to `crates/openai-protocol`.

## Why the fork exists

The gateway deserializes a request into a typed struct and re-serializes it to the selected worker
(`src/routers/http/router.rs`, `send_typed_request`). Any JSON field that the struct does not declare
is dropped in that hop, so slim cannot reach fields that SGLang itself supports. Responses are
proxied as raw bytes and need no change.

## Changes on top of upstream

All three are in `crates/openai-protocol`.

### `GenerateRequest` (`/generate`)

Added, matching `GenerateReqInput` in SGLang 0.5.15.post1:

- `return_routed_experts`
- `routed_experts_start_len`
- `return_prompt_token_ids`
- `return_processor_outputs` (slim's addition to SGLang, not upstream SGLang)

Multimodal processor output already survives the hop: it travels inside `image_data` as
`{"format": "processor_output", ...}`, which is typed `Option<Value>` and therefore opaque.

### `ChatCompletionRequest` (`/v1/chat/completions`)

Added, matching `ChatCompletionRequest` in SGLang 0.5.15.post1:

- `return_prompt_token_ids`
- `return_meta_info`
- `return_routed_experts`
- `routed_experts_start_len`
- `return_cached_tokens_details`
- `return_processor_outputs` (slim's addition to SGLang, not upstream SGLang)

Every field is `Option<T>` with `skip_serializing_if`, so a worker only ever receives the flags the
client actually sent. That keeps non-SGLang backends (vLLM) untouched.

`top_logprobs` drops the OpenAI cap of 20. Score centering can now read
the sampler's top-K over chat completions.

### `FunctionCallResponse::arguments`

Assistant tool calls may now carry `arguments` as a JSON object as well as the OpenAI JSON string.
Objects are normalized to their compact JSON string, so the field type stays `Option<String>` for
every consumer of the struct. SGLang accepts both forms and parses the string back into an object
before applying the chat template (`normalize_assistant_tool_call_arguments` in `serving_chat.py`).

## Build

```bash
cd bindings/python
uv run --no-project --with maturin maturin build --release --features vendored-openssl --out ../../dist
```

`make python-build` does the same when maturin is installed globally.

`uv run python patch_dependencies.py` in slim no longer needs to patch the gateway; it installs this
wheel instead.

## Release

`.github/workflows/release.yml` builds one linux x86_64 wheel tagged `manylinux_2_28`,
so it runs anywhere with glibc 2.28 or newer.
A local build instead carries the build host's glibc, so it is only good for verification.
The container needs `protoc` because `smg-grpc-client` compiles protos in its `build.rs`
and `prost-build` bundles none, and `perl-core`/`make`/`gcc` because `vendored-openssl`
builds OpenSSL from source.
The workflow then checks the metadata with twine, imports the wheel in a clean venv, and
attaches it to a GitHub release.

Publishing a version:

1. Bump `version` in `bindings/python/pyproject.toml` and
   `bindings/python/src/sglang_router/version.py`, raising the `.post<n>` counter. Reset it
   to `.post1` when the upstream gateway version changes.
2. Tag it `v<version>`. The build fails if the tag and the wheel version disagree.
3. slim installs the release asset by URL:

```
https://github.com/weixiao-zhan/sgl-model-gateway/releases/download/v0.3.2.post2/sglang_router-0.3.2.post2-cp38-abi3-manylinux_2_28_x86_64.whl
```

## Resync with upstream

History is shaped for this: the first commit is the untouched upstream import, the second is
the untouched vendored crate, and everything of ours is one commit on top. So `git diff` of
that last commit is the complete patch to carry forward.

1. Import the target `sgl-project/sglang` tag's `sgl-model-gateway/` as a fresh commit on a
   branch, the same way the first commit was made.
2. Read the `openai-protocol` version the new `Cargo.toml` pins, download that version
   (`https://static.crates.io/crates/openai-protocol/openai-protocol-<version>.crate`) and
   commit it under `crates/openai-protocol` minus `Cargo.toml.orig` and `Cargo.lock`.
3. Cherry-pick our delta commit, resolve conflicts, regenerate the root `Cargo.lock`
   (committed here for reproducible release builds, though upstream ships none), then
   `cargo test -p openai-protocol`, `make python-build`, and bump `.post<n>`.
