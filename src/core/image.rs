//! Image resize pipeline for multimodal inputs.
//!
//! Decodes raw image bytes, resizes to fit dimension/pixel budget, then
//! progressively reduces quality and dimension to stay under the byte ceiling.
//! Modeled on letta-code's sharp-backed pipeline.

use std::io::Write;

use base64::Engine;

use crate::core::config::ImageConfig;

/// Progressive resize pipeline for inbound images.
pub struct ResizePipeline {
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u32,
    pub max_bytes: usize,
    pub jpeg_quality: u8,
}

impl ResizePipeline {
    pub fn from_config(config: &ImageConfig) -> Self {
        Self {
            max_width: config.max_width,
            max_height: config.max_height,
            max_pixels: config.max_pixels,
            max_bytes: config.max_bytes,
            jpeg_quality: config.jpeg_quality,
        }
    }

    /// Process raw image bytes into a resized (media_type, base64_data) pair.
    /// Returns None if the image cannot be decoded at all.
    pub fn process(&self, data: &[u8], media_type: &str) -> Option<(String, String)> {
        let img = image::load_from_memory(data).ok()?;
        let img = self.resize_to_fit(&img);

        // Progressive quality reduction, then dimension reduction
        let qualities: &[u8] = &[self.jpeg_quality, 70, 55, 40];
        let scales: &[f64] = &[1.0, 0.75, 0.5, 0.25];

        for &quality in qualities {
            for &scale in scales {
                let scaled = if scale < 1.0 {
                    let w = (img.width() as f64 * scale) as u32;
                    let h = (img.height() as f64 * scale) as u32;
                    img.resize(w.max(64), h.max(64), image::imageops::FilterType::Lanczos3)
                } else {
                    img.clone()
                };
                if let Some((bytes, mime)) = self.encode(&scaled, quality) {
                    if bytes.len() <= self.max_bytes {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        return Some((mime, b64));
                    }
                }
            }
        }

        // Fallback: smallest possible encode even if over budget
        let smallest = img.resize(64, 64, image::imageops::FilterType::Lanczos3);
        if let Some((bytes, mime)) = self.encode(&smallest, 40) {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Some((mime, b64));
        }

        None
    }

    /// Resize an image to fit within max dimensions and pixel budget.
    fn resize_to_fit(&self, img: &image::DynamicImage) -> image::DynamicImage {
        let (w, h) = (img.width(), img.height());
        let scale = self.rescale_factor(w, h);
        if scale < 1.0 {
            let nw = (w as f64 * scale).max(64.0) as u32;
            let nh = (h as f64 * scale).max(64.0) as u32;
            img.resize(nw, nh, image::imageops::FilterType::Lanczos3)
        } else {
            img.clone()
        }
    }

    /// Calculate rescale factor to stay within pixel budget and max dims.
    fn rescale_factor(&self, w: u32, h: u32) -> f64 {
        let mut factor = 1.0_f64;
        if w > self.max_width || h > self.max_height {
            let sx = self.max_width as f64 / w as f64;
            let sy = self.max_height as f64 / h as f64;
            factor = factor.min(sx).min(sy);
        }
        let pixels = (w as f64 * factor) * (h as f64 * factor);
        if pixels > self.max_pixels as f64 {
            factor *= (self.max_pixels as f64 / pixels).sqrt();
        }
        factor
    }

    /// Encode the image — JPEG with adjustable quality, then PNG fallback.
    fn encode(&self, img: &image::DynamicImage, quality: u8) -> Option<(Vec<u8>, String)> {
        // JPEG with quality control
        let mut jpeg_buf = Vec::new();
        let mut jpeg_writer = std::io::Cursor::new(&mut jpeg_buf);
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_writer, quality);
        let rgb = img.to_rgb8();
        if encoder.encode(&rgb, rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8).is_ok() {
            return Some((jpeg_buf, "image/jpeg".to_string()));
        }
        // PNG fallback (lossless, no quality param)
        let mut png_buf = std::io::Cursor::new(Vec::new());
        if img.write_to(&mut png_buf, image::ImageFormat::Png).is_ok() {
            return Some((png_buf.into_inner(), "image/png".to_string()));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resize_pipeline_small_png() {
        // Create a small 1x1 red pixel PNG bytes
        let px = image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 255]).unwrap();
        let mut buf = std::io::Cursor::new(Vec::new());
        px.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let data = buf.into_inner();

        let pipeline = ResizePipeline {
            max_width: 2000,
            max_height: 2000,
            max_pixels: 25_000_000,
            max_bytes: 5_000_000,
            jpeg_quality: 85,
        };

        let result = pipeline.process(&data, "image/png");
        assert!(result.is_some(), "pipeline should process a valid PNG");
        let (mime, b64) = result.unwrap();
        assert_eq!(mime, "image/jpeg");
        assert!(!b64.is_empty(), "base64 output should not be empty");
    }

    #[test]
    fn test_rescale_factor_within_bounds() {
        let pipeline = ResizePipeline {
            max_width: 2000,
            max_height: 2000,
            max_pixels: 25_000_000,
            max_bytes: 5_000_000,
            jpeg_quality: 85,
        };
        let factor = pipeline.rescale_factor(1024, 768);
        assert!((factor - 1.0).abs() < 0.01, "small image should not be scaled");
    }

    #[test]
    fn test_rescale_factor_oversized() {
        let pipeline = ResizePipeline {
            max_width: 2000,
            max_height: 2000,
            max_pixels: 25_000_000,
            max_bytes: 5_000_000,
            jpeg_quality: 85,
        };
        let factor = pipeline.rescale_factor(4000, 3000);
        assert!(factor < 0.6, "oversized image should be scaled down");
    }
}
