use std::collections::BTreeSet;
use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;

use base64::Engine as _;
use pactrail_tools::ToolDescriptor;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Current schema of Pactrail's provider-neutral conversation and request IR.
pub const MODEL_IR_SCHEMA_VERSION: u32 = 1;

/// Maximum number of image artifacts accepted on one user turn.
pub const MAX_INPUT_IMAGES: usize = 4;
/// Maximum decoded bytes accepted for one image artifact.
pub const MAX_INPUT_IMAGE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum aggregate decoded image bytes accepted on one user turn.
pub const MAX_TOTAL_INPUT_IMAGE_BYTES: usize = 12 * 1024 * 1024;
/// Common inline request envelope enforced by all first-party adapters.
pub const MAX_INLINE_MODEL_REQUEST_BYTES: usize = 20 * 1024 * 1024;
/// Maximum width or height accepted for an image artifact.
pub const MAX_INPUT_IMAGE_DIMENSION: u32 = 8_000;
const MAX_INPUT_IMAGE_PIXELS: u64 = 64_000_000;
const MAX_INPUT_IMAGE_NAME_BYTES: usize = 255;

/// Participant role in Pactrail's provider-neutral message representation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Provider-neutral text message.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    /// Creates a system instruction.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    /// Creates a user message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    /// Creates an assistant message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// Provider-neutral image type supported by every first-party vision adapter.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub enum ImageMediaType {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/webp")]
    WebP,
}

impl ImageMediaType {
    /// Returns the fixed IANA media type sent to providers.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::WebP => "image/webp",
        }
    }
}

/// Integrity-bound inline image whose host path has been deliberately erased.
#[derive(Clone, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageArtifact {
    name: String,
    media_type: ImageMediaType,
    data_base64: Arc<str>,
    digest: String,
    bytes: u64,
    width: u32,
    height: u32,
}

impl fmt::Debug for ImageArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImageArtifact")
            .field("name", &self.name)
            .field("media_type", &self.media_type)
            .field("digest", &self.digest)
            .field("bytes", &self.bytes)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish_non_exhaustive()
    }
}

impl ImageArtifact {
    /// Validates and seals one complete local image byte string.
    ///
    /// The file extension is intentionally ignored. Pactrail recognizes the
    /// container signature, validates its bounded structure and dimensions,
    /// and records a BLAKE3 digest before provider transport.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported, malformed, oversized, or dangerously
    /// dimensioned input, or for a name that could disclose a host path.
    pub fn from_bytes(name: impl Into<String>, data: &[u8]) -> Result<Self, ImageArtifactError> {
        let name = name.into();
        validate_image_name(&name)?;
        if data.is_empty() || data.len() > MAX_INPUT_IMAGE_BYTES {
            return Err(ImageArtifactError::InvalidSize {
                actual: data.len(),
                limit: MAX_INPUT_IMAGE_BYTES,
            });
        }
        let (media_type, width, height) = inspect_image(data)?;
        validate_dimensions(width, height)?;
        Ok(Self {
            name,
            media_type,
            data_base64: base64::engine::general_purpose::STANDARD
                .encode(data)
                .into(),
            digest: blake3::hash(data).to_hex().to_string(),
            bytes: u64::try_from(data.len()).unwrap_or(u64::MAX),
            width,
            height,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn media_type(&self) -> ImageMediaType {
        self.media_type
    }

    #[must_use]
    pub fn data_base64(&self) -> &str {
        &self.data_base64
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Conservative provider-neutral input-token reservation.
    ///
    /// This follows a 768-pixel tiling envelope and intentionally rounds up.
    #[must_use]
    pub fn estimated_input_tokens(&self) -> u64 {
        let horizontal = u64::from(self.width).div_ceil(768);
        let vertical = u64::from(self.height).div_ceil(768);
        horizontal.saturating_mul(vertical).saturating_mul(258)
    }
}

impl<'de> Deserialize<'de> for ImageArtifact {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireImageArtifact {
            name: String,
            media_type: ImageMediaType,
            data_base64: String,
            digest: String,
            bytes: u64,
            width: u32,
            height: u32,
        }

        let wire = WireImageArtifact::deserialize(deserializer)?;
        let maximum_encoded = MAX_INPUT_IMAGE_BYTES.div_ceil(3).saturating_mul(4);
        if wire.data_base64.len() > maximum_encoded {
            return Err(serde::de::Error::custom(
                "encoded image exceeds its safety bound",
            ));
        }
        let data = base64::engine::general_purpose::STANDARD
            .decode(&wire.data_base64)
            .map_err(serde::de::Error::custom)?;
        let rebuilt = Self::from_bytes(wire.name, &data).map_err(serde::de::Error::custom)?;
        if rebuilt.media_type != wire.media_type
            || rebuilt.digest != wire.digest
            || rebuilt.bytes != wire.bytes
            || rebuilt.width != wire.width
            || rebuilt.height != wire.height
        {
            return Err(serde::de::Error::custom(
                "image metadata or digest does not match the sealed bytes",
            ));
        }
        Ok(rebuilt)
    }
}

/// One user instruction with explicitly attached, integrity-bound images.
#[derive(Clone, Debug, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UserContent {
    pub text: String,
    pub images: Vec<ImageArtifact>,
}

impl<'de> Deserialize<'de> for UserContent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct WireUserContent {
            text: String,
            images: Vec<ImageArtifact>,
        }

        let wire = WireUserContent::deserialize(deserializer)?;
        Self::new(wire.text, wire.images).map_err(serde::de::Error::custom)
    }
}

impl UserContent {
    /// Creates validated multimodal user content.
    ///
    /// # Errors
    ///
    /// Returns an error when the image set exceeds aggregate bounds or contains
    /// duplicate artifacts.
    pub fn new(
        text: impl Into<String>,
        images: Vec<ImageArtifact>,
    ) -> Result<Self, ImageArtifactError> {
        let text = text.into();
        if text.is_empty() && images.is_empty() {
            return Err(ImageArtifactError::EmptyUserContent);
        }
        validate_image_set(&images)?;
        Ok(Self { text, images })
    }
}

/// Aggregate facts used to reserve transport and context capacity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageSetSummary {
    pub count: usize,
    pub total_bytes: u64,
    pub estimated_input_tokens: u64,
}

/// Validates one complete image set and returns its bounded resource totals.
///
/// # Errors
///
/// Returns an error for too many images, duplicate digests, or excess bytes.
pub fn validate_image_set(images: &[ImageArtifact]) -> Result<ImageSetSummary, ImageArtifactError> {
    if images.len() > MAX_INPUT_IMAGES {
        return Err(ImageArtifactError::TooManyImages {
            actual: images.len(),
            limit: MAX_INPUT_IMAGES,
        });
    }
    let mut digests = BTreeSet::new();
    let mut total_bytes = 0_u64;
    let mut estimated_input_tokens = 0_u64;
    for image in images {
        if !digests.insert(image.digest.as_str()) {
            return Err(ImageArtifactError::Duplicate(image.digest.clone()));
        }
        total_bytes = total_bytes.saturating_add(image.bytes);
        estimated_input_tokens =
            estimated_input_tokens.saturating_add(image.estimated_input_tokens());
    }
    if total_bytes > MAX_TOTAL_INPUT_IMAGE_BYTES as u64 {
        return Err(ImageArtifactError::TotalSize {
            actual: total_bytes,
            limit: MAX_TOTAL_INPUT_IMAGE_BYTES as u64,
        });
    }
    Ok(ImageSetSummary {
        count: images.len(),
        total_bytes,
        estimated_input_tokens,
    })
}

pub(crate) fn validate_request_images(
    conversation: &[ConversationItem],
    vision: bool,
) -> Result<(), String> {
    let mut found = false;
    for item in conversation {
        if let ConversationItem::UserContent(content) = item {
            validate_image_set(&content.images).map_err(|error| error.to_string())?;
            found |= !content.images.is_empty();
        }
    }
    if found && !vision {
        Err("the configured model profile does not declare vision support".to_owned())
    } else {
        Ok(())
    }
}

pub(crate) fn validate_request_body_size(body: &Value) -> Result<(), String> {
    #[derive(Default)]
    struct ByteCounter(usize);

    impl Write for ByteCounter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0 = self.0.saturating_add(buffer.len());
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, body)
        .map_err(|error| format!("failed to measure provider request: {error}"))?;
    let bytes = counter.0;
    if bytes > MAX_INLINE_MODEL_REQUEST_BYTES {
        Err(format!(
            "provider request has {bytes} bytes; Pactrail's portable inline limit is {MAX_INLINE_MODEL_REQUEST_BYTES}"
        ))
    } else {
        Ok(())
    }
}

/// Image sealing or set-validation failure.
#[derive(Debug, Error)]
pub enum ImageArtifactError {
    #[error("multimodal user content must contain text or at least one image")]
    EmptyUserContent,
    #[error("image name must be a bounded filename, never a host path")]
    InvalidName,
    #[error("image has {actual} bytes; expected 1..={limit}")]
    InvalidSize { actual: usize, limit: usize },
    #[error("image container is unsupported or structurally malformed: {0}")]
    InvalidContainer(&'static str),
    #[error("image dimensions {width}x{height} exceed the safety envelope")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("{actual} images were provided; at most {limit} are allowed")]
    TooManyImages { actual: usize, limit: usize },
    #[error("image set has {actual} decoded bytes; at most {limit} are allowed")]
    TotalSize { actual: u64, limit: u64 },
    #[error("image digest {0} was attached more than once")]
    Duplicate(String),
}

fn validate_image_name(name: &str) -> Result<(), ImageArtifactError> {
    if name.is_empty()
        || name.len() > MAX_INPUT_IMAGE_NAME_BYTES
        || name.contains(['/', '\\', '\0', '\r', '\n'])
        || name.chars().any(char::is_control)
        || matches!(name, "." | "..")
    {
        Err(ImageArtifactError::InvalidName)
    } else {
        Ok(())
    }
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), ImageArtifactError> {
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if width == 0
        || height == 0
        || width > MAX_INPUT_IMAGE_DIMENSION
        || height > MAX_INPUT_IMAGE_DIMENSION
        || pixels > MAX_INPUT_IMAGE_PIXELS
    {
        Err(ImageArtifactError::InvalidDimensions { width, height })
    } else {
        Ok(())
    }
}

fn inspect_image(data: &[u8]) -> Result<(ImageMediaType, u32, u32), ImageArtifactError> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        inspect_png(data).map(|(width, height)| (ImageMediaType::Png, width, height))
    } else if data.starts_with(&[0xff, 0xd8]) {
        inspect_jpeg(data).map(|(width, height)| (ImageMediaType::Jpeg, width, height))
    } else if data.starts_with(b"RIFF") {
        inspect_webp(data).map(|(width, height)| (ImageMediaType::WebP, width, height))
    } else {
        Err(ImageArtifactError::InvalidContainer(
            "expected PNG, JPEG, or WebP signature",
        ))
    }
}

fn inspect_png(data: &[u8]) -> Result<(u32, u32), ImageArtifactError> {
    let invalid = || ImageArtifactError::InvalidContainer("invalid PNG chunk structure");
    if data.len() < 45 {
        return Err(invalid());
    }
    let mut cursor = 8_usize;
    let mut dimensions = None;
    while cursor.checked_add(12).is_some_and(|end| end <= data.len()) {
        let length = read_be_u32(data, cursor).ok_or_else(invalid)? as usize;
        let chunk_end = cursor
            .checked_add(12)
            .and_then(|base| base.checked_add(length))
            .filter(|end| *end <= data.len())
            .ok_or_else(invalid)?;
        let kind = &data[cursor + 4..cursor + 8];
        if dimensions.is_none() {
            if kind != b"IHDR" || length != 13 {
                return Err(invalid());
            }
            dimensions = Some((
                read_be_u32(data, cursor + 8).ok_or_else(invalid)?,
                read_be_u32(data, cursor + 12).ok_or_else(invalid)?,
            ));
        }
        cursor = chunk_end;
        if kind == b"IEND" {
            return if length == 0 && cursor == data.len() {
                dimensions.ok_or_else(invalid)
            } else {
                Err(invalid())
            };
        }
    }
    Err(invalid())
}

fn inspect_jpeg(data: &[u8]) -> Result<(u32, u32), ImageArtifactError> {
    let invalid = || ImageArtifactError::InvalidContainer("invalid JPEG marker structure");
    if data.len() < 12 || !data.ends_with(&[0xff, 0xd9]) {
        return Err(invalid());
    }
    let mut cursor = 2_usize;
    while cursor < data.len() {
        while data.get(cursor) == Some(&0xff) {
            cursor = cursor.saturating_add(1);
        }
        let marker = *data.get(cursor).ok_or_else(invalid)?;
        cursor = cursor.saturating_add(1);
        if marker == 0x00 || marker == 0xd8 || (0xd0..=0xd9).contains(&marker) {
            continue;
        }
        let length = usize::from(read_be_u16(data, cursor).ok_or_else(invalid)?);
        if length < 2 {
            return Err(invalid());
        }
        let segment_end = cursor
            .checked_add(length)
            .filter(|end| *end <= data.len())
            .ok_or_else(invalid)?;
        if is_jpeg_start_of_frame(marker) {
            if length < 8 {
                return Err(invalid());
            }
            let height = u32::from(read_be_u16(data, cursor + 3).ok_or_else(invalid)?);
            let width = u32::from(read_be_u16(data, cursor + 5).ok_or_else(invalid)?);
            return Ok((width, height));
        }
        if marker == 0xda {
            return Err(invalid());
        }
        cursor = segment_end;
    }
    Err(invalid())
}

const fn is_jpeg_start_of_frame(marker: u8) -> bool {
    matches!(
        marker,
        0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf
    )
}

fn inspect_webp(data: &[u8]) -> Result<(u32, u32), ImageArtifactError> {
    let invalid = || ImageArtifactError::InvalidContainer("invalid WebP RIFF structure");
    if data.len() < 30 || data.get(8..12) != Some(b"WEBP") {
        return Err(invalid());
    }
    let declared = read_le_u32(data, 4).ok_or_else(invalid)? as usize;
    if declared.checked_add(8) != Some(data.len()) {
        return Err(invalid());
    }
    let mut cursor = 12_usize;
    while cursor.checked_add(8).is_some_and(|end| end <= data.len()) {
        let kind = data.get(cursor..cursor + 4).ok_or_else(invalid)?;
        let length = read_le_u32(data, cursor + 4).ok_or_else(invalid)? as usize;
        let payload = cursor + 8;
        let chunk_end = payload
            .checked_add(length)
            .filter(|end| *end <= data.len())
            .ok_or_else(invalid)?;
        let dimensions = match kind {
            b"VP8X" if length >= 10 => Some((
                1 + read_le_u24(data, payload + 4).ok_or_else(invalid)?,
                1 + read_le_u24(data, payload + 7).ok_or_else(invalid)?,
            )),
            b"VP8L" if length >= 5 && data.get(payload) == Some(&0x2f) => {
                let b1 = u32::from(data[payload + 1]);
                let b2 = u32::from(data[payload + 2]);
                let b3 = u32::from(data[payload + 3]);
                let b4 = u32::from(data[payload + 4]);
                Some((
                    1 + b1 + ((b2 & 0x3f) << 8),
                    1 + (b2 >> 6) + (b3 << 2) + ((b4 & 0x0f) << 10),
                ))
            }
            b"VP8 "
                if length >= 10
                    && data.get(payload + 3..payload + 6) == Some(&[0x9d, 0x01, 0x2a]) =>
            {
                Some((
                    u32::from(read_le_u16(data, payload + 6).ok_or_else(invalid)? & 0x3fff),
                    u32::from(read_le_u16(data, payload + 8).ok_or_else(invalid)? & 0x3fff),
                ))
            }
            _ => None,
        };
        if let Some(dimensions) = dimensions {
            return Ok(dimensions);
        }
        cursor = chunk_end.saturating_add(length % 2);
    }
    Err(invalid())
}

fn read_be_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_be_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn read_le_u16(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn read_le_u24(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset + 3)?;
    Some(u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16))
}

fn read_le_u32(data: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        data.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// Tool invocation requested by a model.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
    /// Non-sensitive provider continuation metadata, such as a signed
    /// function-call thought token that must be echoed on the next turn.
    #[serde(default)]
    pub extensions: serde_json::Map<String, Value>,
}

/// Tool result returned to a model on the next turn.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ToolResult {
    pub call_id: String,
    pub name: String,
    pub content: Value,
    pub is_error: bool,
}

/// One durable item in the provider-neutral model conversation.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ConversationItem {
    Message(Message),
    UserContent(UserContent),
    AssistantToolCalls { text: String, calls: Vec<ToolCall> },
    ToolResult(ToolResult),
}

/// Declared capabilities of one configured model endpoint.
// Provider capabilities are intentionally independent feature flags rather than
// mutually exclusive states.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(default)]
pub struct ModelCapabilities {
    pub native_tools: bool,
    pub parallel_tools: bool,
    pub structured_output: bool,
    pub vision: bool,
    pub prompt_caching: bool,
    pub streaming: bool,
    pub reasoning_controls: bool,
    pub context_tokens: u64,
    pub max_output_tokens: u64,
    pub source: CapabilitySource,
}

/// Provenance of the effective model capability profile.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    #[default]
    ConservativeDefault,
    UserDeclared,
    Probed,
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            native_tools: true,
            parallel_tools: false,
            structured_output: false,
            vision: false,
            prompt_caching: false,
            streaming: false,
            reasoning_controls: false,
            context_tokens: 32_768,
            max_output_tokens: 4_096,
            source: CapabilitySource::ConservativeDefault,
        }
    }
}

/// Complete normalized request passed to a model driver.
#[derive(Clone, Debug)]
pub struct ModelRequest {
    pub conversation: Vec<ConversationItem>,
    pub tools: Vec<ToolDescriptor>,
    pub max_output_tokens: u64,
    pub temperature: Option<f32>,
}

/// Reason a model stopped producing output.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Complete,
    ToolCalls,
    Length,
    ContentFilter,
    Unknown,
}

/// Normalized token accounting from a provider.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
}

impl Usage {
    /// Adds another turn's counters without integer overflow.
    #[must_use]
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
            cached_input_tokens: self
                .cached_input_tokens
                .saturating_add(other.cached_input_tokens),
        }
    }

    /// Total reported input and output tokens.
    #[must_use]
    pub const fn total(self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// Complete normalized response from a model driver.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ModelResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage: Usage,
    pub provider_request_id: Option<String>,
    /// Non-sensitive provider metadata preserved for diagnostics.
    pub extensions: serde_json::Map<String, Value>,
}

/// Transient, non-authoritative progress from one model response stream.
///
/// These events are suitable for live user interfaces. Pactrail does not
/// persist them or allow partial tool arguments to reach the tool kernel.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ModelStreamEvent {
    /// The provider accepted the request and response bytes began arriving.
    ResponseStarted {
        provider_request_id: Option<String>,
        time_to_first_byte_ms: u64,
    },
    /// A validated UTF-8 assistant-text fragment.
    TextDelta { text: String },
    /// A typed tool call began. Arguments are not yet executable.
    ToolCallStarted {
        index: usize,
        id: String,
        name: String,
    },
    /// Additional JSON argument bytes arrived for an in-progress tool call.
    ToolArgumentsDelta { index: usize, bytes: usize },
    /// Provider-reported cumulative usage became available.
    UsageUpdate { usage: Usage },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::test_support::tiny_png;

    fn tiny_jpeg(width: u16, height: u16) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xc0, 0x00, 0x0b, 8];
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&[1, 1, 0x11, 0]);
        bytes.extend_from_slice(&[0xff, 0xd9]);
        bytes
    }

    fn tiny_webp(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&22_u32.to_le_bytes());
        bytes.extend_from_slice(b"WEBPVP8X");
        bytes.extend_from_slice(&10_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        let width = width.saturating_sub(1).to_le_bytes();
        let height = height.saturating_sub(1).to_le_bytes();
        bytes.extend_from_slice(&width[..3]);
        bytes.extend_from_slice(&height[..3]);
        bytes
    }

    #[test]
    fn seals_the_cross_provider_image_formats() {
        for (name, bytes, media_type, dimensions) in [
            (
                "screen.png",
                tiny_png(640, 480),
                ImageMediaType::Png,
                (640, 480),
            ),
            (
                "photo.bin",
                tiny_jpeg(320, 200),
                ImageMediaType::Jpeg,
                (320, 200),
            ),
            (
                "capture.webp",
                tiny_webp(1_024, 768),
                ImageMediaType::WebP,
                (1_024, 768),
            ),
        ] {
            let artifact = ImageArtifact::from_bytes(name, &bytes)
                .unwrap_or_else(|error| unreachable!("seal image: {error}"));
            assert_eq!(artifact.media_type(), media_type);
            assert_eq!((artifact.width(), artifact.height()), dimensions);
            assert_eq!(artifact.bytes(), bytes.len() as u64);
            assert_eq!(artifact.digest().len(), 64);
            assert!(!format!("{artifact:?}").contains(artifact.data_base64()));
        }
    }

    #[test]
    fn rejects_paths_malformed_containers_and_dangerous_dimensions() {
        assert!(matches!(
            ImageArtifact::from_bytes("C:\\secret.png", &tiny_png(1, 1)),
            Err(ImageArtifactError::InvalidName)
        ));
        let mut trailing = tiny_png(1, 1);
        trailing.push(0);
        assert!(matches!(
            ImageArtifact::from_bytes("bad.png", &trailing),
            Err(ImageArtifactError::InvalidContainer(_))
        ));
        assert!(matches!(
            ImageArtifact::from_bytes("huge.png", &tiny_png(8_001, 1)),
            Err(ImageArtifactError::InvalidDimensions { .. })
        ));
    }

    #[test]
    fn deserialization_revalidates_the_digest_and_metadata() {
        let artifact = ImageArtifact::from_bytes("screen.png", &tiny_png(10, 20))
            .unwrap_or_else(|error| unreachable!("seal image: {error}"));
        let mut value = serde_json::to_value(&artifact)
            .unwrap_or_else(|error| unreachable!("serialize: {error}"));
        value["width"] = json!(99);
        let Err(error) = serde_json::from_value::<ImageArtifact>(value) else {
            unreachable!("tampered metadata must fail")
        };
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn image_sets_reject_duplicates_and_excess_count() {
        let artifact = ImageArtifact::from_bytes("screen.png", &tiny_png(10, 20))
            .unwrap_or_else(|error| unreachable!("seal image: {error}"));
        assert!(matches!(
            validate_image_set(&[artifact.clone(), artifact.clone()]),
            Err(ImageArtifactError::Duplicate(_))
        ));
        let images = (0..=MAX_INPUT_IMAGES)
            .map(|index| {
                let width =
                    u32::try_from(index).unwrap_or_else(|error| unreachable!("width: {error}")) + 1;
                ImageArtifact::from_bytes(format!("{index}.png"), &tiny_png(width, 1))
                    .unwrap_or_else(|error| unreachable!("seal image: {error}"))
            })
            .collect::<Vec<_>>();
        assert!(matches!(
            validate_image_set(&images),
            Err(ImageArtifactError::TooManyImages { .. })
        ));
    }

    #[test]
    fn serialized_provider_request_has_a_portable_hard_limit() {
        assert!(validate_request_body_size(&json!({"text": "small"})).is_ok());
        let oversized = json!({"text": "x".repeat(MAX_INLINE_MODEL_REQUEST_BYTES)});
        assert!(validate_request_body_size(&oversized).is_err());
    }
}
