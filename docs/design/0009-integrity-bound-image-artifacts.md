# Design 0009: integrity-bound image artifacts

Status: implemented for 0.8

## Problem

Coding tasks increasingly begin with screenshots, rendered regressions, design
references, diagrams, and visual error output. Passing an arbitrary path or URL
straight to one provider would violate Pactrail's portability and trust model:
the model could receive host identity, a driver could perform hidden network
fetches, a file could change before resume, transport encoding could consume an
unbounded request, and traces could accidentally retain sensitive payloads.

The feature therefore has to behave like every other Pactrail input: explicit,
bounded, integrity-bound, provider-neutral, resumable, and observable without
leaking the evidence itself.

## Decision

Pactrail defines a sealed `ImageArtifact` and an ordered multimodal
`ConversationItem::UserContent`. The portable format intersection is PNG, JPEG,
and WebP. A task may contain at most four images, each at most 4 MiB decoded and
12 MiB decoded in aggregate. Dimensions must be non-zero, at most 8,000 pixels
on either edge, and at most 64 million pixels.

The CLI accepts repeatable `--image PATH` and an interactive next-task queue:

```text
/image add screenshot.png
/image list
/image clear
```

`/capability vision on` or `--vision on` remains a separate assertion about the
chosen endpoint/model. Pactrail never infers model vision support merely from a
provider label.

## Sealing boundary

Before any run directory, transaction, event, or network request is created,
the CLI:

1. rejects symlinks and non-regular files;
2. checks the opened file against the pre-open size and reads through a hard
   limit;
3. discards every path component except one bounded UTF-8 filename;
4. recognizes the byte signature and parses bounded PNG/JPEG/WebP header and
   container structure without trusting the extension;
5. validates dimensions and set-wide limits;
6. rejects duplicate complete-byte BLAKE3 digests; and
7. base64-encodes the complete bytes into the sealed artifact.

The constructor and custom deserializer apply the same validation. A checkpoint
whose base64, media type, byte count, dimensions, or digest disagree fails to
deserialize. Debug output deliberately omits base64.

## Provider mapping

The image bytes remain inline. Pactrail does not fetch remote image URLs, follow
image redirects, or create implicit provider file uploads.

- OpenAI-compatible Chat Completions receives text parts and an
  `image_url.url` base64 data URL.
- Anthropic Messages receives labelled base64 `image` source blocks followed by
  the task text.
- Gemini GenerateContent receives task/label text parts and `inlineData`.

Every request builder rejects image content when its effective
`ModelCapabilities::vision` is false and rejects a serialized request above the
portable 20 MiB inline ceiling. These shapes follow the providers'
primary documentation: [OpenAI images and
vision](https://developers.openai.com/api/docs/guides/images-vision),
[Anthropic vision](https://platform.claude.com/docs/en/build-with-claude/vision),
and [Gemini GenerateContent image
understanding](https://ai.google.dev/gemini-api/docs/generate-content/image-understanding).

## Context and performance

Base64 transport bytes are not text tokens. Serializing them into Pactrail's
text-window controller would therefore reject useful small images for the wrong
reason. Context fingerprints replace each base64 value with its sealed digest,
while the engine subtracts a conservative 258-token-per-768-pixel-tile estimate
from declared context before repository context compilation and trajectory
budgeting. If the image reservation consumes the available input window, the
run fails before model access.

The base64 field uses shared immutable ownership. Conversation and request
clones copy a pointer for the payload rather than duplicating up to 12 MiB each
turn. Provider serialization still emits the full required request body.

## Durability and observability

Images live in the initial provider-neutral user turn. The complete sealed turn
is stored in the existing compressed content-addressed checkpoint and bound to
the hash-linked event head. Resume restores those exact bytes and refuses new
attachments, so it never reopens the original mutable path.

The live timeline emits the image count, total decoded bytes, conservative token
reservation, and digest prefixes. The durable action journal records the same
bounded metadata. Neither trace contains host paths or base64. The filename and
pixels are labelled untrusted evidence in the system policy.

## Failure and recovery

Unsupported data, malformed bounded structure, dangerous dimensions, excess
count/bytes, duplicates, a text-only capability profile, or an impossible
context budget fail before provider invocation. A provider may still reject a
structurally recognized image it cannot decode; that is a normal explicit model
request failure and the preceding checkpoint remains authoritative.

Inline images are resent on each model turn by all three protocol families.
This costs latency and provider tokens, but avoids provider-owned upload
lifecycle, hidden network state, and non-portable file identifiers in v1.

## Rejected alternatives

- **Pass local paths to drivers:** leaks host identity and makes resume depend on
  mutable external state.
- **Accept remote URLs:** creates implicit network authority, redirects, DNS
  ambiguity, and non-reproducible bytes.
- **Provider file APIs:** introduces upload/delete lifecycle, provider-specific
  identifiers, and recovery state before the portable contract is stable.
- **Trust extensions or MIME declarations:** permits type confusion.
- **Count base64 as text context:** conflates wire size with visual-token cost.
- **Persist only a digest:** prevents restart after the source file changes or
  disappears.

## Verification

Tests cover PNG/JPEG/WebP recognition, extension independence, path/name
erasure, malformed/trailing data, dangerous dimensions, duplicate/count bounds,
metadata/digest tampering, payload-redacted debug output, all three provider
wire shapes, vision-profile rejection, context fingerprint redaction, local
checkpoint round trips, repeatable CLI parsing, and local loader behavior. The
normal workspace test, lint, documentation, and cross-platform release gates
remain authoritative.
