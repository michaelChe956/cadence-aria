use image::{ImageError, ImageFormat, Limits};
use std::io::Cursor;
use thiserror::Error;

pub const MAX_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_SIDE: u32 = 4096;

const MAX_PIXELS: u64 = MAX_SIDE as u64 * MAX_SIDE as u64;
const MAX_ALLOC: u64 = MAX_PIXELS * 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedImage {
    pub media_type: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RefImageError {
    #[error("reference image exceeds the 10 MiB size limit")]
    TooLarge,
    #[error("reference image MIME type is unsupported")]
    UnsupportedMime,
    #[error("reference image content does not match its declared MIME type")]
    InvalidFormat,
    #[error("reference image could not be decoded")]
    Decoding,
    #[error("animated or multi-frame reference images are unsupported")]
    AnimatedOrMultiFrame,
    #[error("reference image dimensions or pixel count exceed the limit")]
    TooManyPixels,
}

pub fn validate_reference_image(
    bytes: &[u8],
    declared_mime: &str,
) -> Result<ValidatedImage, RefImageError> {
    if bytes.len() > MAX_BYTES {
        return Err(RefImageError::TooLarge);
    }

    let declared_format = mime_to_format(declared_mime).ok_or(RefImageError::UnsupportedMime)?;

    #[allow(deprecated)]
    let mut reader = image::io::Reader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| RefImageError::Decoding)?;
    let actual_format = reader.format().ok_or(RefImageError::Decoding)?;
    if actual_format != declared_format {
        return Err(RefImageError::InvalidFormat);
    }

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);

    let decoded = match reader.decode() {
        Ok(decoded) => decoded,
        Err(error) if is_dimension_limit_error(&error) => {
            return Err(RefImageError::TooManyPixels);
        }
        Err(_) => return Err(RefImageError::Decoding),
    };

    reject_animation(bytes, actual_format)?;

    let width = decoded.width();
    let height = decoded.height();
    if width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width).saturating_mul(u64::from(height)) > MAX_PIXELS
    {
        return Err(RefImageError::TooManyPixels);
    }

    Ok(ValidatedImage {
        media_type: declared_mime.to_owned(),
        width,
        height,
    })
}

fn mime_to_format(declared_mime: &str) -> Option<ImageFormat> {
    match declared_mime {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

fn reject_animation(bytes: &[u8], format: ImageFormat) -> Result<(), RefImageError> {
    match format {
        ImageFormat::WebP => {
            let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes))
                .map_err(|_| RefImageError::Decoding)?;
            if decoder.has_animation() {
                return Err(RefImageError::AnimatedOrMultiFrame);
            }
        }
        ImageFormat::Png => {
            let reader = png::Decoder::new(Cursor::new(bytes))
                .read_info()
                .map_err(|_| RefImageError::Decoding)?;
            if reader.info().animation_control.is_some() {
                return Err(RefImageError::AnimatedOrMultiFrame);
            }
        }
        ImageFormat::Jpeg => {}
        _ => {}
    }

    Ok(())
}

fn is_dimension_limit_error(error: &ImageError) -> bool {
    matches!(
        error,
        ImageError::Limits(limit)
            if matches!(limit.kind(), image::error::LimitErrorKind::DimensionError)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::PngEncoder;
    use image::codecs::webp::WebPEncoder;
    use image::{ExtendedColorType, ImageEncoder};
    use std::io::Cursor;

    const WIDTH: u32 = 2;
    const HEIGHT: u32 = 3;

    fn encode_png(width: u32, height: u32) -> Vec<u8> {
        let pixels = vec![0x7f; (width * height) as usize];
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(&pixels, width, height, ExtendedColorType::L8)
            .unwrap();
        bytes
    }

    fn encode_jpeg(width: u32, height: u32) -> Vec<u8> {
        let pixels = vec![0x7f; (width * height) as usize];
        let mut bytes = Vec::new();
        JpegEncoder::new(&mut bytes)
            .write_image(&pixels, width, height, ExtendedColorType::L8)
            .unwrap();
        bytes
    }

    fn encode_webp(width: u32, height: u32) -> Vec<u8> {
        let pixels = vec![0x7f; (width * height * 3) as usize];
        let mut bytes = Vec::new();
        WebPEncoder::new_lossless(&mut bytes)
            .write_image(&pixels, width, height, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    fn encode_apng() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, WIDTH, HEIGHT);
            encoder.set_color(png::ColorType::Grayscale);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(2, 0).unwrap();
            let mut writer = encoder.write_header().unwrap();
            let frame = vec![0x7f; (WIDTH * HEIGHT) as usize];
            writer.write_image_data(&frame).unwrap();
            writer.write_image_data(&frame).unwrap();
            writer.finish().unwrap();
        }
        bytes
    }

    fn push_webp_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        output.extend_from_slice(kind);
        output.extend_from_slice(&(data.len() as u32).to_le_bytes());
        output.extend_from_slice(data);
        if !data.len().is_multiple_of(2) {
            output.push(0);
        }
    }

    fn encode_animated_webp() -> Vec<u8> {
        let static_webp = encode_webp(WIDTH, HEIGHT);
        let image_chunk = &static_webp[12..];

        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(b"WEBP");

        let mut extended_header = vec![0b0000_0010, 0, 0, 0];
        extended_header.extend_from_slice(&(WIDTH - 1).to_le_bytes()[..3]);
        extended_header.extend_from_slice(&(HEIGHT - 1).to_le_bytes()[..3]);
        push_webp_chunk(&mut bytes, b"VP8X", &extended_header);
        push_webp_chunk(&mut bytes, b"ANIM", &[0, 0, 0, 0, 0, 0]);

        let mut frame = vec![0; 6];
        frame.extend_from_slice(&(WIDTH - 1).to_le_bytes()[..3]);
        frame.extend_from_slice(&(HEIGHT - 1).to_le_bytes()[..3]);
        frame.extend_from_slice(&[1, 0, 0, 0]);
        frame.extend_from_slice(image_chunk);
        push_webp_chunk(&mut bytes, b"ANMF", &frame);

        let riff_size = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        bytes
    }

    #[test]
    fn valid_png_is_accepted() {
        let result = validate_reference_image(&encode_png(WIDTH, HEIGHT), "image/png").unwrap();

        assert_eq!(
            result,
            ValidatedImage {
                media_type: "image/png".to_owned(),
                width: WIDTH,
                height: HEIGHT,
            }
        );
    }

    #[test]
    fn valid_jpeg_is_accepted() {
        let result = validate_reference_image(&encode_jpeg(WIDTH, HEIGHT), "image/jpeg").unwrap();

        assert_eq!(
            result,
            ValidatedImage {
                media_type: "image/jpeg".to_owned(),
                width: WIDTH,
                height: HEIGHT,
            }
        );
    }

    #[test]
    fn valid_webp_is_accepted() {
        let result = validate_reference_image(&encode_webp(WIDTH, HEIGHT), "image/webp").unwrap();

        assert_eq!(
            result,
            ValidatedImage {
                media_type: "image/webp".to_owned(),
                width: WIDTH,
                height: HEIGHT,
            }
        );
    }

    #[test]
    fn payload_larger_than_ten_megabytes_is_rejected_first() {
        let bytes = vec![0; MAX_BYTES + 1];

        assert_eq!(
            validate_reference_image(&bytes, "image/gif"),
            Err(RefImageError::TooLarge)
        );
    }

    #[test]
    fn image_exceeding_side_limit_is_rejected() {
        let bytes = encode_png(MAX_SIDE + 1, 1);

        assert_eq!(
            validate_reference_image(&bytes, "image/png"),
            Err(RefImageError::TooManyPixels)
        );
    }

    #[test]
    fn declared_mime_must_match_decoded_format() {
        let bytes = encode_jpeg(WIDTH, HEIGHT);

        assert_eq!(
            validate_reference_image(&bytes, "image/png"),
            Err(RefImageError::InvalidFormat)
        );
    }

    #[test]
    fn disguised_gif_is_rejected_as_invalid_format_before_decode() {
        const ONE_PIXEL_GIF: &[u8] = &[
            0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00,
            0x00, 0x00, 0xff, 0xff, 0xff, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
            0x00, 0x02, 0x01, 0x4c, 0x00, 0x3b,
        ];

        assert_eq!(
            validate_reference_image(ONE_PIXEL_GIF, "image/png"),
            Err(RefImageError::InvalidFormat)
        );
    }

    #[test]
    fn gif_mime_is_unsupported_without_decoding() {
        let gif_header = b"GIF89a";

        assert_eq!(
            validate_reference_image(gif_header, "image/gif"),
            Err(RefImageError::UnsupportedMime)
        );
    }

    #[test]
    fn malformed_image_is_rejected_as_decoding_error() {
        assert_eq!(
            validate_reference_image(b"not a png", "image/png"),
            Err(RefImageError::Decoding)
        );
    }

    #[test]
    fn apng_is_rejected_as_multiframe() {
        assert_eq!(
            validate_reference_image(&encode_apng(), "image/png"),
            Err(RefImageError::AnimatedOrMultiFrame)
        );
    }

    #[test]
    fn animated_webp_is_rejected_as_multiframe() {
        let bytes = encode_animated_webp();
        let decoder = image::codecs::webp::WebPDecoder::new(Cursor::new(&bytes)).unwrap();
        assert!(decoder.has_animation(), "fixture must be an animated WebP");

        assert_eq!(
            validate_reference_image(&bytes, "image/webp"),
            Err(RefImageError::AnimatedOrMultiFrame)
        );
    }
}
