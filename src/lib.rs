//! Pure Rust HEIC/HEIF image decoder
//!
//! This crate provides a safe, sandboxed HEIC image decoder without
//! any C/C++ dependencies.
//!
//! # Example
//!
//! ```ignore
//! use heic_decoder::HeicDecoder;
//!
//! let data = std::fs::read("image.heic")?;
//! let decoder = HeicDecoder::new();
//! let image = decoder.decode(&data)?;
//! println!("Decoded {}x{} image", image.width, image.height);
//! ```

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Allow dead code during development - will be removed when decoder is complete
#![allow(dead_code)]

extern crate alloc;

mod error;
pub mod heif;
pub mod hevc;

pub use error::{HeicError, Result};

use alloc::vec::Vec;

/// Decoded image data
#[derive(Debug, Clone)]
pub struct DecodedImage {
    /// Raw pixel data in RGB or RGBA format
    pub data: Vec<u8>,
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
    /// Whether the image has an alpha channel
    pub has_alpha: bool,
}

/// Image metadata without full decode
#[derive(Debug, Clone, Copy)]
pub struct ImageInfo {
    /// Image width in pixels
    pub width: u32,
    /// Image height in pixels
    pub height: u32,
    /// Whether the image has an alpha channel
    pub has_alpha: bool,
}

/// HEIC image decoder
#[derive(Debug, Default)]
pub struct HeicDecoder {
    _private: (),
}

impl HeicDecoder {
    /// Create a new HEIC decoder
    #[must_use]
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Decode HEIC data to raw pixels
    ///
    /// # Errors
    ///
    /// Returns an error if the data is not valid HEIC/HEIF format
    /// or if decoding fails.
    pub fn decode(&self, data: &[u8]) -> Result<DecodedImage> {
        // Parse HEIF container
        let container = heif::parse(data)?;

        // Find primary image item
        let primary_item = container.primary_item().ok_or(HeicError::NoPrimaryImage)?;

        // Workaround for grid images: decode the first tile instead
        // TODO: implement proper grid decoding with tile stitching
        let item = if primary_item.item_type == heif::ItemType::Grid {
            // Find the first HEVC tile item (usually item_id=1)
            let tile_info = container.item_infos.iter()
                .find(|info| {
                    let item_type: heif::ItemType = info.item_type.into();
                    item_type == heif::ItemType::Hvc1
                })
                .ok_or(HeicError::InvalidData("Grid has no tile items"))?;
            container.get_item(tile_info.item_id).ok_or(HeicError::InvalidData("Tile item not found"))?
        } else {
            primary_item
        };

        // Get image data
        let image_data = container
            .get_item_data(item.id)
            .ok_or(HeicError::InvalidData("Missing image data"))?;

        // Decode HEVC using config if available
        let frame = if let Some(ref config) = item.hevc_config {
            hevc::decode_with_config(config, image_data)?
        } else {
            // Fallback to raw decode (Annex B or self-contained)
            hevc::decode(image_data)?
        };

        Ok(DecodedImage {
            data: frame.to_rgb(),
            width: frame.cropped_width(),
            height: frame.cropped_height(),
            has_alpha: false, // TODO: handle alpha plane
        })
    }

    /// Decode HEIC data to raw YCbCr frame (for debugging)
    ///
    /// # Errors
    ///
    /// Returns an error if the data is not valid HEIC/HEIF format.
    pub fn decode_to_frame(&self, data: &[u8]) -> Result<hevc::DecodedFrame> {
        let container = heif::parse(data)?;
        let primary_item = container.primary_item().ok_or(HeicError::NoPrimaryImage)?;
        
        // Workaround for grid images: decode the first tile instead
        // TODO: implement proper grid decoding with tile stitching
        let (item_id, item_type) = if primary_item.item_type == heif::ItemType::Grid {
            // Find the first HEVC tile item (usually item_id=1)
            let tile_item = container.item_infos.iter()
                .find(|info| {
                    let item_type: heif::ItemType = info.item_type.into();
                    item_type == heif::ItemType::Hvc1
                })
                .ok_or(HeicError::InvalidData("Grid has no tile items"))?;
            (tile_item.item_id, heif::ItemType::Hvc1)
        } else {
            (primary_item.id, primary_item.item_type)
        };
        
        let item = container.get_item(item_id).ok_or(HeicError::InvalidData("Item not found"))?;
        let image_data = container
            .get_item_data(item.id)
            .ok_or(HeicError::InvalidData("Missing image data"))?;

        if let Some(ref config) = item.hevc_config {
            Ok(hevc::decode_with_config(config, image_data)?)
        } else {
            Ok(hevc::decode(image_data)?)
        }
    }

    /// Get image info without full decoding
    ///
    /// # Errors
    ///
    /// Returns an error if the data is not valid HEIC/HEIF format.
    pub fn get_info(&self, data: &[u8]) -> Result<ImageInfo> {
        let container = heif::parse(data)?;

        let primary_item = container.primary_item().ok_or(HeicError::NoPrimaryImage)?;

        // Try to get info from HEVC config first (faster, no mdat access needed)
        if let Some(ref config) = primary_item.hevc_config
            && let Ok(info) = hevc::get_info_from_config(config)
        {
            return Ok(ImageInfo {
                width: info.width,
                height: info.height,
                has_alpha: false,
            });
        }

        // Fallback to reading image data
        let image_data = container
            .get_item_data(primary_item.id)
            .ok_or(HeicError::InvalidData("Missing image data"))?;

        let info = hevc::get_info(image_data)?;

        Ok(ImageInfo {
            width: info.width,
            height: info.height,
            has_alpha: false,
        })
    }
}
