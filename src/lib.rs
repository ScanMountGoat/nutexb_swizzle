use ahash::AHashMap;
use binread::prelude::*;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::{
    io::{Cursor, Write},
    path::Path,
};

use crate::swizzle::{swizzle_x_16, swizzle_x_8, swizzle_y_16, swizzle_y_8};

mod nutexb;
mod swizzle;

pub enum ImageFormat {
    Rgba8,
    RgbaF32,
    Bc1,
    Bc3,
    Bc7,
}

/// The necessary trait bounds for types that can be used for swizzle calculation functions.
/// The [u32], [u64], and [u128] types implement the necessary traits and can be used to represent block sizes of 4, 8, and 16 bytes, respectively.
pub trait LookupBlock:
    BinRead + Eq + PartialEq + Default + Copy + Send + Sync + std::hash::Hash
{
}
impl<T: BinRead + Eq + PartialEq + Default + Copy + Send + Sync + std::hash::Hash> LookupBlock
    for T
{
}

pub fn swizzle_data(
    input_data: &[u8],
    width: usize,
    height: usize,
    format: &ImageFormat,
) -> Vec<u8> {
    let width_in_blocks = width / 4;
    let height_in_blocks = height / 4;

    let tile_size = get_tile_size(format);

    let mut output_data = vec![0u8; width_in_blocks * height_in_blocks * tile_size];
    // TODO: Support other formats.
    match format {
        ImageFormat::Rgba8 => {}
        ImageFormat::Bc1 => swizzle::swizzle_experimental(
            swizzle_x_8,
            swizzle_y_8,
            width_in_blocks,
            height_in_blocks,
            &input_data,
            &mut output_data[..],
            false,
            8,
        ),
        ImageFormat::Bc3 | ImageFormat::Bc7 => swizzle::swizzle_experimental(
            swizzle_x_16,
            swizzle_y_16,
            width_in_blocks,
            height_in_blocks,
            &input_data,
            &mut output_data[..],
            false,
            16,
        ),
        ImageFormat::RgbaF32 => swizzle::swizzle_experimental(
            swizzle_x_16,
            swizzle_y_16,
            width,
            height,
            &input_data,
            &mut output_data[..],
            false,
            16,
        ),
    }

    output_data
}

pub fn swizzle<P: AsRef<Path>>(
    input: P,
    output: P,
    width: usize,
    height: usize,
    format: &ImageFormat,
) {
    let input_data = std::fs::read(input).unwrap();
    let output_data = swizzle_data(&input_data, width, height, format);

    let mut writer = std::fs::File::create(output).unwrap();
    for value in output_data {
        writer.write_all(&value.to_le_bytes()).unwrap();
    }
}

pub fn deswizzle_data(
    input_data: &[u8],
    width: usize,
    height: usize,
    format: &ImageFormat,
) -> Vec<u8> {
    // TODO: This isn't correct for RGBA.
    let width_in_blocks = width / 4;
    let height_in_blocks = height / 4;

    let tile_size = get_tile_size(format);

    let mut output_data = vec![0u8; width_in_blocks * height_in_blocks * tile_size];
    // TODO: Support other formats.
    match format {
        // TODO: This can just be based on block size rather than image format.
        ImageFormat::Rgba8 => {}
        ImageFormat::Bc1 => swizzle::swizzle_experimental(
            swizzle_x_8,
            swizzle_y_8,
            width_in_blocks,
            height_in_blocks,
            &input_data,
            &mut output_data[..],
            true,
            8,
        ),
        ImageFormat::Bc3 | ImageFormat::Bc7 => swizzle::swizzle_experimental(
            swizzle_x_16,
            swizzle_y_16,
            width_in_blocks,
            height_in_blocks,
            &input_data,
            &mut output_data[..],
            true,
            16,
        ),
        ImageFormat::RgbaF32 => swizzle::swizzle_experimental(
            swizzle_x_16,
            swizzle_y_16,
            width,
            height,
            &input_data,
            &mut output_data[..],
            true,
            16,
        ),
    }

    output_data
}

// TODO: Avoid repetitive code.
pub fn deswizzle<P: AsRef<Path>>(
    input: P,
    output: P,
    width: usize,
    height: usize,
    format: &ImageFormat,
) {
    let input_data = std::fs::read(input).unwrap();
    let output_data = deswizzle_data(&input_data, width, height, format);

    let mut writer = std::fs::File::create(output).unwrap();
    for value in output_data {
        writer.write_all(&value.to_le_bytes()).unwrap();
    }
}

pub fn try_get_image_format(format: &str) -> std::result::Result<ImageFormat, &str> {
    match format {
        "rgba8" => Ok(ImageFormat::Rgba8),
        "rgbaf32" => Ok(ImageFormat::RgbaF32),
        "bc1" => Ok(ImageFormat::Bc1),
        "bc3" => Ok(ImageFormat::Bc3),
        "bc7" => Ok(ImageFormat::Bc7),
        _ => Err("Unsupported format"),
    }
}

fn get_tile_size(format: &ImageFormat) -> usize {
    match format {
        ImageFormat::Rgba8 => 4,
        ImageFormat::RgbaF32 => 16,
        ImageFormat::Bc1 => 8,
        ImageFormat::Bc3 | ImageFormat::Bc7 => 16,
    }
}

fn read_vec<T: BinRead, R: BinReaderExt>(reader: &mut R) -> Vec<T> {
    let mut result = Vec::new();
    while let Ok(block) = reader.read_le::<T>() {
        result.push(block);
    }
    result
}

fn read_blocks<P: AsRef<Path>, T: BinRead>(path: P) -> Vec<T> {
    let mut raw = Cursor::new(std::fs::read(path).unwrap());
    read_vec(&mut raw)
}

fn read_mipmaps_dds<P: AsRef<Path>, T: BinRead>(path: P) -> Vec<Vec<T>> {
    let mut reader = std::fs::File::open(path).unwrap();
    let dds = ddsfile::Dds::read(&mut reader).unwrap();

    // Each mip level is 4x smaller than the previous level.
    let mut mip_offset = 0;
    let mut mip_size = dds.get_main_texture_size().unwrap() as usize;
    let min_mipmap_size = dds.get_min_mipmap_size_in_bytes() as usize;

    let mut mip_data = Vec::new();
    for _ in 0..dds.get_num_mipmap_levels() {
        let mut reader = Cursor::new(&dds.data[mip_offset..mip_offset + mip_size]);
        let blocks = read_vec(&mut reader);
        mip_data.push(blocks);

        // Some compressed formats have a minimum size.
        mip_offset += std::cmp::max(mip_size, min_mipmap_size);
        mip_size /= 4;
    }

    mip_data
}

fn create_deswizzle_luts<T: LookupBlock>(
    linear_mipmaps: &[Vec<T>],
    deswizzled_mipmaps: &[Vec<T>],
) -> Vec<Vec<i64>> {
    let mut luts = Vec::new();

    for (linear_mip, deswizzled_mip) in deswizzled_mipmaps.iter().zip(linear_mipmaps) {
        let mip_lut = create_mip_deswizzle_lut(linear_mip, deswizzled_mip);
        luts.push(mip_lut);
    }

    luts
}

fn create_mip_deswizzle_lut<T: LookupBlock>(linear: &[T], deswizzled: &[T]) -> Vec<i64> {
    // For each deswizzled output block index, find the corresponding input block index.
    // The lookup table allows for iterating the input lists only once for an O(n) running time.
    let mut linear_index_by_block = AHashMap::with_capacity(linear.len());
    for (i, value) in linear.iter().enumerate() {
        linear_index_by_block.insert(value, i);
    }

    deswizzled
        .par_iter()
        .map(|block| {
            linear_index_by_block
                .get(block)
                .map(|i| *i as i64)
                .unwrap_or(-1)
        })
        .collect()
}

// TODO: Return result?
pub fn write_rgba_lut<W: Write>(writer: &mut W, pixel_count: usize) {
    for i in 0..pixel_count as u32 {
        // Use the linear address to create unique pixel values.
        writer.write_all(&i.to_le_bytes()).unwrap();
    }
}

pub fn write_rgba_f32_lut<W: Write>(writer: &mut W, pixel_count: usize) {
    for i in 0..pixel_count {
        // Use the linear address to create unique pixel values.
        // TODO: This only works up to 16777216.
        // TODO: Flip sign bit for larger values?
        writer.write_all(&(i as f32).to_le_bytes()).unwrap();
        writer.write_all(&0f32.to_le_bytes()).unwrap();
        writer.write_all(&0f32.to_le_bytes()).unwrap();
        writer.write_all(&0f32.to_le_bytes()).unwrap();
    }
}

pub fn write_bc7_lut<W: Write>(writer: &mut W, block_count: usize) {
    for i in 0..block_count as u64 {
        // Create 128 bits of unique BC7 data.
        // We just need unique blocks rather than unique pixel colors.
        writer.write_all(&0u32.to_le_bytes()).unwrap();
        writer.write_all(&i.to_le_bytes()).unwrap();
        writer.write_all(&2u32.to_le_bytes()).unwrap();
    }
}

pub fn write_bc3_lut<W: Write>(writer: &mut W, block_count: usize) {
    for i in 0..block_count as u64 {
        // Create 128 bits of unique BC3 data.
        // We just need unique blocks rather than unique pixel colors.
        writer.write_all(&65535u64.to_le_bytes()).unwrap();
        writer.write_all(&i.to_le_bytes()).unwrap();
    }
}

pub fn write_bc1_lut<W: Write>(writer: &mut W, block_count: usize) {
    for i in 0..block_count as u32 {
        // Create 64 bits of unique BC1 data.
        // We just need unique blocks rather than unique pixel colors.
        writer.write_all(&0u32.to_le_bytes()).unwrap();
        writer.write_all(&i.to_le_bytes()).unwrap();
    }
}

fn get_swizzle_patterns_output(
    deswizzle_lut: &[i64],
    width: usize,
    height: usize,
    tile_dimension: usize,
) -> String {
    if width == 0 || height == 0 || deswizzle_lut.is_empty() {
        return String::new();
    }

    let width_in_tiles = width / tile_dimension;
    let height_in_tiles = height / tile_dimension;

    let x_pattern_index = if width_in_tiles > 1 {
        width_in_tiles - 1
    } else {
        0
    };
    let y_pattern_index = if height_in_tiles > 1 {
        width_in_tiles * (height_in_tiles - 1)
    } else {
        0
    };

    return format!(
        "width: {:?}, height: {:?}\nx: {:032b}\ny: {:032b}",
        width, height, deswizzle_lut[x_pattern_index], deswizzle_lut[y_pattern_index]
    );
}

fn get_mipmap_range(lut: &[i64]) -> (i64, i64) {
    (*lut.iter().min().unwrap(), *lut.iter().max().unwrap())
}

pub fn guess_swizzle_patterns<T: LookupBlock, P: AsRef<Path>>(
    swizzled_file: P,
    deswizzled_file: P,
    width: usize,
    height: usize,
    format: &ImageFormat,
) {
    let swizzled_mipmaps = match std::path::Path::new(swizzled_file.as_ref())
        .extension()
        .unwrap()
        .to_str()
        .unwrap()
    {
        "dds" => read_mipmaps_dds(&swizzled_file),
        _ => vec![read_blocks::<_, T>(&swizzled_file)],
    };

    let deswizzled_mipmaps = match std::path::Path::new(deswizzled_file.as_ref())
        .extension()
        .unwrap()
        .to_str()
        .unwrap()
    {
        "dds" => read_mipmaps_dds(&deswizzled_file),
        _ => vec![read_blocks::<_, T>(&deswizzled_file)],
    };

    // TODO: There is a lot of repetition for these two conditions.
    if swizzled_mipmaps.len() == 1 && deswizzled_mipmaps.len() > 1 {
        // Associate each mipmap with its mip level to avoid having to use enumerate with rayon.
        let deswizzled_mipmaps: Vec<_> = deswizzled_mipmaps.iter().enumerate().collect();

        // The mipmaps can now be computed independently.
        // Collect will ensure the outputs are still displayed in the expected order.
        let mip_outputs: Vec<_> = deswizzled_mipmaps
            .par_iter()
            .map(|(i, mip)| {
                // TODO: Is this necessary for all formats?
                let mip_width = width / (2usize.pow(*i as u32));
                let mip_height = height / (2usize.pow(*i as u32));
                if mip_width < 4 || mip_height < 4 {
                    return String::new();
                }

                // Assume the input blocks cover all mip levels.
                // This allows for calculating mip offsets and sizes based on the range of block indices.
                let mut mip_lut = create_mip_deswizzle_lut(&swizzled_mipmaps[0], &mip);
                let (start_index, end_index) = get_mipmap_range(&mip_lut);

                // For the swizzle patterns, assume the swizzling starts from the mipmap offset.
                for val in mip_lut.iter_mut() {
                    *val -= start_index;
                }

                let tile_dimension = match format {
                    ImageFormat::Rgba8 => 1,
                    _ => 4,
                };
                let swizzle_output =
                    get_swizzle_patterns_output(&mip_lut, mip_width, mip_height, tile_dimension);

                format!(
                    "Start Index: {:?}\nEnd Index: {:?}\n{}\n",
                    start_index, end_index, swizzle_output
                )
            })
            .collect();

        for output in mip_outputs {
            println!("{}", output);
        }
    } else {
        // Compare both mipmaps.
        let mip_luts = create_deswizzle_luts(&swizzled_mipmaps, &deswizzled_mipmaps);
        let mip_luts: Vec<_> = mip_luts.iter().enumerate().collect();
        // TODO: This can also be done in parallel.
        let mip_outputs: Vec<_> = mip_luts
            .iter()
            .map(|(i, mip_lut)| {
                // TODO: Is this necessary for all formats?
                let mip_width = width / (2usize.pow(*i as u32));
                let mip_height = height / (2usize.pow(*i as u32));
                if mip_width < 4 || mip_height < 4 {
                    return String::new();
                }

                let tile_dimension = match format {
                    ImageFormat::Rgba8 => 1,
                    _ => 4,
                };
                get_swizzle_patterns_output(&mip_lut, mip_width, mip_height, tile_dimension)
            })
            .collect();

        for output in mip_outputs {
            println!("{}", output);
        }
    }
}

pub fn create_nutexb<W: Write>(
    writer: &mut W,
    width: usize,
    height: usize,
    name: &str,
    format: &ImageFormat,
    block_count: usize,
) {
    let nutexb_format = match format {
        ImageFormat::Rgba8 => 0,
        ImageFormat::Bc1 => 128,
        ImageFormat::Bc3 => 160,
        ImageFormat::Bc7 => 224,
        ImageFormat::RgbaF32 => 52,
    };

    let mut buffer = Cursor::new(Vec::new());
    match format {
        ImageFormat::Rgba8 => write_rgba_lut(&mut buffer, block_count),
        ImageFormat::Bc1 => write_bc1_lut(&mut buffer, block_count),
        ImageFormat::Bc3 => write_bc3_lut(&mut buffer, block_count),
        ImageFormat::Bc7 => write_bc7_lut(&mut buffer, block_count),
        ImageFormat::RgbaF32 => write_rgba_f32_lut(&mut buffer, block_count),
    }

    nutexb::write_nutexb_from_data(
        writer,
        buffer.get_ref(),
        width as u32,
        height as u32,
        name,
        nutexb_format,
    )
    .unwrap();
}
