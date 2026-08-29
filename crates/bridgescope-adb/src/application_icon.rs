//! Launcher icon extraction from an installed APK.
//!
//! ADB offers no "read app icon" command, so the icon travels in three
//! device-side reads per application: the APK's ZIP central directory (tail
//! of the file), `AndroidManifest.xml` (to find the icon resource id), and
//! `resources.arsc` (to resolve that id to an image file). The actual icon
//! bytes are then fetched with a `dd` byte range — a few hundred kilobytes
//! instead of pulling whole APKs.
//!
//! Everything here is best-effort: any device or parse failure yields no
//! icon, and the UI keeps its fallback tile.

use std::{ffi::OsString, io::Read, path::Path, time::Duration};

use bridgescope_domain::{ApplicationIconData, BridgeError, DeviceSerial, ErrorCode, PackageName};

use crate::process::run_bounded;

const BLOCK: u64 = 4096;
const TAIL_BYTES: u64 = 256 * 1024;
const CENTRAL_DIR_LIMIT: u64 = 64 * 1024 * 1024;
const ENTRY_LIMIT: u64 = 32 * 1024 * 1024;
const STDERR_LIMIT: usize = 16 * 1024;
const EOCD_MAGIC: u32 = 0x0605_4b50;
const CENTRAL_MAGIC: u32 = 0x0201_4b50;
const LOCAL_MAGIC: u32 = 0x0403_4b50;
const METHOD_STORED: u16 = 0;
const METHOD_DEFLATED: u16 = 8;
/// Deliver icons at a size the grid can use without further scaling.
const MAX_ICON_DIMENSION: u32 = 128;

// ---------------------------------------------------------------- readers

async fn shell_text(
    executable: &Path,
    serial: &DeviceSerial,
    command: &[&str],
    timeout: Duration,
) -> Result<String, BridgeError> {
    let mut arguments: Vec<OsString> = vec![
        OsString::from("-s"),
        OsString::from(serial.as_str()),
        OsString::from("shell"),
    ];
    arguments.extend(command.iter().map(OsString::from));
    let output = run_bounded(executable, arguments, timeout, 1024 * 1024, STDERR_LIMIT).await?;
    if output.exit_code != Some(0) {
        return Err(BridgeError::new(
            ErrorCode::AdbFailed,
            "adb.command_failed",
            "shell command failed while reading icon data",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

async fn exec_out_bytes(
    executable: &Path,
    serial: &DeviceSerial,
    command: &[&str],
    limit: usize,
    timeout: Duration,
) -> Result<Vec<u8>, BridgeError> {
    let mut arguments: Vec<OsString> = vec![
        OsString::from("-s"),
        OsString::from(serial.as_str()),
        OsString::from("exec-out"),
    ];
    arguments.extend(command.iter().map(OsString::from));
    let output = run_bounded(executable, arguments, timeout, limit, STDERR_LIMIT).await?;
    if output.exit_code != Some(0) {
        return Err(BridgeError::new(
            ErrorCode::AdbFailed,
            "adb.command_failed",
            "exec-out command failed while reading icon data",
        ));
    }
    Ok(output.stdout)
}

/// Reads `len` bytes at `offset` from a device file. The primary path is a
/// byte-exact `toybox dd`: some images ship `/system/bin/dd` as a broken
/// symlink into `toolbox` (which lacks the applet) and their toybox `dd`
/// also miscomputes block `skip=`, so byte-exact flags sidestep both. When
/// toybox is unavailable (older boxes) the plain block-aligned `dd` runs.
/// A legit read here is always several kilobytes, so a tiny reply means the
/// tool printed an error instead of data.
async fn read_range(
    executable: &Path,
    serial: &DeviceSerial,
    path: &str,
    offset: u64,
    len: u64,
    timeout: Duration,
) -> Result<Vec<u8>, BridgeError> {
    if len == 0 {
        return Ok(Vec::new());
    }
    // `pm path` output never contains quotes, so single-quoting keeps the
    // device-side `sh` from touching anything inside the path; the trailing
    // redirect keeps dd's transfer stats off the binary stream. The whole
    // device command travels as ONE adb argument: multi-argument commands
    // get re-quoted differently across adb builds and arrive mangled.
    let byte_cmd = format!(
        "toybox dd if='{path}' bs=65536 skip={offset} count={len} \
         iflag=skip_bytes,count_bytes 2>/dev/null"
    );
    if let Ok(raw) = exec_out_bytes(
        executable,
        serial,
        &[&byte_cmd],
        usize::try_from(len.saturating_add(4096)).unwrap_or(usize::MAX),
        timeout,
    )
    .await
        && raw.len() > 64
    {
        return Ok(raw);
    }

    let skip = offset / BLOCK;
    let count = (offset % BLOCK + len).div_ceil(BLOCK);
    let legacy_cmd = format!("dd if='{path}' bs={BLOCK} skip={skip} count={count} 2>/dev/null");
    let limit =
        usize::try_from(count.saturating_mul(BLOCK).saturating_add(BLOCK)).unwrap_or(usize::MAX);
    let raw = exec_out_bytes(executable, serial, &[&legacy_cmd], limit, timeout).await?;
    let head = usize::try_from(offset % BLOCK).unwrap_or(0);
    Ok(raw
        .into_iter()
        .skip(head)
        .take(usize::try_from(len).unwrap_or(usize::MAX))
        .collect())
}

/// Resolves the installed APK path and its size in ONE shell round trip;
/// every saved round trip directly speeds up the per-app icon pipeline.
async fn apk_path_and_size(
    executable: &Path,
    serial: &DeviceSerial,
    package: &PackageName,
    timeout: Duration,
) -> Option<(String, u64)> {
    let package = package.as_str();
    let script = format!(
        "f=$(pm path {package} | sed -n 's/^package://p' | grep base.apk | head -n 1); \
         [ -n \"$f\" ] || f=$(pm path {package} | sed -n 's/^package://p' | head -n 1); \
         echo \"path:$f\"; stat -c %s \"$f\" 2>/dev/null || wc -c < \"$f\""
    );
    let output = shell_text(executable, serial, &[&script], timeout)
        .await
        .ok()?;
    parse_path_and_size(&output)
}

/// Host-side parse of the combined `path:` line plus size output.
fn parse_path_and_size(output: &str) -> Option<(String, u64)> {
    let mut path = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("path:") {
            if path.is_none() && !rest.is_empty() {
                path = Some(rest.to_owned());
            }
        } else if let Ok(size) = line.parse::<u64>()
            && let Some(path) = path
        {
            return Some((path, size));
        }
    }
    None
}

async fn apk_size(
    executable: &Path,
    serial: &DeviceSerial,
    path: &str,
    timeout: Duration,
) -> Option<u64> {
    let stat = shell_text(executable, serial, &["stat", "-c", "%s", path], timeout)
        .await
        .ok()?;
    if let Ok(size) = stat.trim().parse::<u64>() {
        return Some(size);
    }
    let wc = shell_text(executable, serial, &["wc", "-c", path], timeout)
        .await
        .ok()?;
    wc.split_whitespace().next()?.parse::<u64>().ok()
}

// ------------------------------------------------------------------ zip

struct ZipEntry {
    name: String,
    method: u16,
    compressed_size: u64,
    local_offset: u64,
}

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Locates the end-of-central-directory record in the APK tail, returning
/// `(central_dir_offset, central_dir_size)`.
fn find_eocd(tail: &[u8]) -> Option<(u64, u64)> {
    let last = tail.len().checked_sub(22)?;
    let mut scan = last;
    loop {
        if u32_at(tail, scan) == Some(EOCD_MAGIC) {
            let cd_size = u64::from(u32_at(tail, scan + 12)?);
            let cd_offset = u64::from(u32_at(tail, scan + 16)?);
            // 0xFFFFFFFF marks ZIP64, which installed APKs never use.
            if cd_offset != u64::from(u32::MAX) && cd_size != u64::from(u32::MAX) {
                return Some((cd_offset, cd_size));
            }
            return None;
        }
        if scan == 0 {
            return None;
        }
        scan -= 1;
    }
}

fn parse_central_directory(directory: &[u8]) -> Option<Vec<ZipEntry>> {
    let mut entries = Vec::new();
    let mut at = 0usize;
    loop {
        if u32_at(directory, at) != Some(CENTRAL_MAGIC) {
            break;
        }
        let method = u16_at(directory, at + 10)?;
        let compressed_size = u64::from(u32_at(directory, at + 20)?);
        let name_len = usize::from(u16_at(directory, at + 28)?);
        let extra_len = usize::from(u16_at(directory, at + 30)?);
        let comment_len = usize::from(u16_at(directory, at + 32)?);
        let local_offset = u64::from(u32_at(directory, at + 42)?);
        let name = directory
            .get(at.checked_add(46)?..at.checked_add(46)?.checked_add(name_len)?)?
            .iter()
            .map(|byte| {
                if byte.is_ascii_graphic() || *byte == b' ' {
                    char::from(*byte)
                } else {
                    char::REPLACEMENT_CHARACTER
                }
            })
            .collect::<String>();
        entries.push(ZipEntry {
            name,
            method,
            compressed_size,
            local_offset,
        });
        at = at
            .checked_add(46)?
            .checked_add(name_len)?
            .checked_add(extra_len)?
            .checked_add(comment_len)?;
    }
    Some(entries)
}

/// Slices and decompresses one entry from a buffer that starts at the
/// entry's local header.
fn entry_data_from_local(buffer: &[u8], entry: &ZipEntry) -> Option<Vec<u8>> {
    if u32_at(buffer, 0) != Some(LOCAL_MAGIC) {
        return None;
    }
    let name_len = usize::from(u16_at(buffer, 26)?);
    let extra_len = usize::from(u16_at(buffer, 28)?);
    let data = buffer.get(30usize.checked_add(name_len)?.checked_add(extra_len)?..)?;
    let wanted = usize::try_from(entry.compressed_size).ok()?;
    let data = data.get(..wanted.min(data.len()))?;
    match entry.method {
        METHOD_STORED => Some(data.to_vec()),
        METHOD_DEFLATED => {
            let mut decoder = flate2::read::DeflateDecoder::new(data);
            let mut out = Vec::new();
            decoder.read_to_end(&mut out).ok()?;
            Some(out)
        }
        _ => None,
    }
}

// ----------------------------------------------------------- string pool

struct StringPool {
    strings: Vec<String>,
}

fn parse_string_pool(chunk: &[u8]) -> Option<StringPool> {
    if u16_at(chunk, 0) != Some(0x0001) {
        return None;
    }
    let count = u32_at(chunk, 8)?;
    let utf8 = u32_at(chunk, 16)? & 0x100 != 0;
    let strings_start = usize::try_from(u32_at(chunk, 20)?).ok()?;
    let mut strings = Vec::new();
    for index in 0..count {
        let at = 28usize.checked_add(usize::try_from(index).ok()?.checked_mul(4)?)?;
        let offset = usize::try_from(u32_at(chunk, at)?).ok()?;
        let start = strings_start.checked_add(offset)?;
        let string = if utf8 {
            let mut cursor = start;
            let _chars = pool_varint8(chunk, &mut cursor)?;
            let bytes = pool_varint8(chunk, &mut cursor)?;
            let text = chunk.get(cursor..cursor.checked_add(bytes)?)?;
            String::from_utf8_lossy(text).into_owned()
        } else {
            let mut cursor = start;
            let chars = pool_varint16(chunk, &mut cursor)?;
            let mut units = Vec::with_capacity(chars);
            for _ in 0..chars {
                units.push(u16_at(chunk, cursor)?);
                cursor = cursor.checked_add(2)?;
            }
            String::from_utf16_lossy(&units)
        };
        strings.push(string);
    }
    Some(StringPool { strings })
}

fn pool_varint8(data: &[u8], cursor: &mut usize) -> Option<usize> {
    let first = usize::from(*data.get(*cursor)?);
    *cursor += 1;
    if first & 0x80 == 0 {
        return Some(first);
    }
    let second = usize::from(*data.get(*cursor)?);
    *cursor += 1;
    Some(((first & 0x7F) << 8) | second)
}

fn pool_varint16(data: &[u8], cursor: &mut usize) -> Option<usize> {
    let first = usize::from(u16_at(data, *cursor)?);
    *cursor += 2;
    if first & 0x8000 == 0 {
        return Some(first);
    }
    let second = usize::from(u16_at(data, *cursor)?);
    *cursor += 2;
    Some(((first & 0x7FFF) << 16) | second)
}

// ------------------------------------------------------------------ axml

struct AxmlElement {
    name: String,
    attributes: Vec<(String, u8, u32)>,
}

/// Collects the start elements of a compiled binary XML with their
/// attribute `(name, dataType, data)` triples.
fn parse_axml(data: &[u8]) -> Option<Vec<AxmlElement>> {
    if u16_at(data, 0) != Some(0x0003) {
        return None;
    }
    let total = usize::try_from(u32_at(data, 4)?).ok()?.min(data.len());
    let mut pool: Option<StringPool> = None;
    let mut elements = Vec::new();
    let mut at = usize::from(u16_at(data, 2)?);
    while at + 8 <= total {
        let chunk_type = u16_at(data, at)?;
        let header_size = usize::from(u16_at(data, at + 2)?);
        let chunk_size = usize::try_from(u32_at(data, at + 4)?).ok()?;
        if chunk_size < 8 || at.saturating_add(chunk_size) > total {
            break;
        }
        let chunk = &data[at..at + chunk_size];
        match chunk_type {
            0x0001 => pool = parse_string_pool(chunk),
            0x0102 => {
                if let Some(strings) = pool.as_ref()
                    && let Some(element) = parse_axml_start(chunk, header_size, strings)
                {
                    elements.push(element);
                }
            }
            _ => {}
        }
        at += chunk_size;
    }
    Some(elements)
}

fn parse_axml_start(chunk: &[u8], header_size: usize, strings: &StringPool) -> Option<AxmlElement> {
    let name_index = u32_at(chunk, 20)?;
    let name = strings
        .strings
        .get(usize::try_from(name_index).ok()?)?
        .clone();
    let attribute_start = usize::from(u16_at(chunk, 24)?);
    let attribute_size = usize::from(u16_at(chunk, 26)?);
    let attribute_count = usize::from(u16_at(chunk, 28)?);
    if attribute_size < 20 {
        return None;
    }
    let mut attributes = Vec::new();
    for index in 0..attribute_count {
        let at = header_size
            .checked_add(attribute_start)?
            .checked_add(index.checked_mul(attribute_size)?)?;
        let name_index = u32_at(chunk, at + 4)?;
        let data_type = *chunk.get(at.checked_add(15)?)?;
        let data = u32_at(chunk, at.checked_add(16)?)?;
        let attribute_name = strings
            .strings
            .get(usize::try_from(name_index).ok()?)
            .cloned()
            .unwrap_or_default();
        attributes.push((attribute_name, data_type, data));
    }
    Some(AxmlElement { name, attributes })
}

/// Reference attribute value, e.g. `android:drawable="@mipmap/ic_launcher"`.
fn axml_reference(
    elements: &[AxmlElement],
    element_name: &str,
    attribute_name: &str,
) -> Option<u32> {
    let element = elements
        .iter()
        .find(|element| element.name == element_name)?;
    let (_, data_type, data) = element
        .attributes
        .iter()
        .find(|(name, _, _)| name == attribute_name)?;
    // Res_value type REFERENCE.
    if *data_type == 0x01 {
        Some(*data)
    } else {
        None
    }
}

// ------------------------------------------------------------------ arsc

#[derive(Clone, Debug)]
enum ResolvedValue {
    /// Path of the resource file inside the APK (`res/mipmap-…/ic.png`).
    File(String),
    /// ARGB color used directly as an icon layer.
    Color(u32),
}

struct ArscType {
    type_id: u8,
    sparse: bool,
    entry_count: u32,
    entries_start: u32,
    density: u16,
    /// The whole type chunk; offsets are resolved against it.
    data: Vec<u8>,
}

struct ArscPackage {
    id: u32,
    types: Vec<ArscType>,
}

struct Arsc {
    values: StringPool,
    packages: Vec<ArscPackage>,
}

fn parse_arsc(data: &[u8]) -> Option<Arsc> {
    if u16_at(data, 0) != Some(0x0002) {
        return None;
    }
    let table_size = usize::try_from(u32_at(data, 4)?).ok()?.min(data.len());
    let mut values: Option<StringPool> = None;
    let mut packages = Vec::new();
    let mut at = usize::from(u16_at(data, 2)?);
    while at + 8 <= table_size {
        let chunk_type = u16_at(data, at)?;
        let header_size = usize::from(u16_at(data, at + 2)?);
        let chunk_size = usize::try_from(u32_at(data, at + 4)?).ok()?;
        if chunk_size < 8 || at.saturating_add(chunk_size) > table_size {
            break;
        }
        let chunk = &data[at..at + chunk_size];
        match chunk_type {
            0x0001 => values = parse_string_pool(chunk),
            0x0200 => packages.push(parse_arsc_package(chunk, header_size)),
            _ => {}
        }
        at += chunk_size;
    }
    Some(Arsc {
        values: values?,
        packages,
    })
}

fn parse_arsc_package(chunk: &[u8], header_size: usize) -> ArscPackage {
    let id = u32_at(chunk, 8).unwrap_or(0);
    let mut types = Vec::new();
    let mut inner = header_size;
    while inner + 8 <= chunk.len() {
        let inner_type = u16_at(chunk, inner).unwrap_or(0);
        let inner_size = usize::try_from(u32_at(chunk, inner + 4).unwrap_or(0)).unwrap_or(0);
        if inner_size < 8 || inner.saturating_add(inner_size) > chunk.len() {
            break;
        }
        if inner_type == 0x0201
            && let Some(entry) = parse_arsc_type(&chunk[inner..inner + inner_size])
        {
            types.push(entry);
        }
        inner += inner_size;
    }
    ArscPackage { id, types }
}

fn parse_arsc_type(chunk: &[u8]) -> Option<ArscType> {
    let type_id = *chunk.get(8)?;
    let flags = *chunk.get(9)?;
    let entry_count = u32_at(chunk, 12)?;
    let entries_start = u32_at(chunk, 16)?;
    // ResTable_config starts right after the 20-byte header; density sits at
    // offset 14 inside it (after orientation and touchscreen).
    let density = u16_at(chunk, 20 + 14)?;
    Some(ArscType {
        type_id,
        sparse: flags & 0x01 != 0,
        entry_count,
        entries_start,
        density,
        data: chunk.to_vec(),
    })
}

fn type_entry_offset(entry_type: &ArscType, entry_index: u32) -> Option<u32> {
    let header_size = usize::from(u16_at(&entry_type.data, 2)?);
    if entry_type.sparse {
        for index in 0..entry_type.entry_count {
            let at = header_size.checked_add(usize::try_from(index).ok()?.checked_mul(4)?)?;
            if u32::from(u16_at(&entry_type.data, at)?) == entry_index {
                // Sparse offsets are stored shifted left by two.
                return Some(u32::from(u16_at(&entry_type.data, at + 2)?) << 2);
            }
        }
        None
    } else {
        let at = header_size.checked_add(usize::try_from(entry_index).ok()?.checked_mul(4)?)?;
        u32_at(&entry_type.data, at)
    }
}

fn type_entry_value(entry_type: &ArscType, offset: u32) -> Option<(u8, u32)> {
    let at = usize::try_from(entry_type.entries_start)
        .ok()?
        .checked_add(usize::try_from(offset).ok()?)?;
    let entry_size = usize::from(u16_at(&entry_type.data, at)?);
    let flags = u16_at(&entry_type.data, at + 2)?;
    if flags & 0x0001 != 0 {
        // Complex/bag entries are never icon resources.
        return None;
    }
    let value_at = at.checked_add(entry_size)?;
    let data_type = *entry_type.data.get(value_at.checked_add(3)?)?;
    let data = u32_at(&entry_type.data, value_at.checked_add(4)?)?;
    Some((data_type, data))
}

/// Resolves one resource id to a file path or color, following reference
/// aliases. Configurations are ranked by density first — the largest usable
/// icon wins — with PNG breaking density ties (it stays lossless).
fn arsc_resolve(arsc: &Arsc, resource_id: u32) -> Option<ResolvedValue> {
    let package_id = resource_id >> 24;
    let type_id = u8::try_from((resource_id >> 16) & 0xFF).ok()?;
    let entry_index = resource_id & 0xFFFF;
    let package = arsc
        .packages
        .iter()
        .find(|package| package.id == package_id)
        .or_else(|| arsc.packages.first())?;
    let mut best: Option<(u32, u32, u8, u32)> = None;
    for entry_type in &package.types {
        if entry_type.type_id != type_id || entry_index >= entry_type.entry_count {
            continue;
        }
        let offset = match type_entry_offset(entry_type, entry_index) {
            Some(offset) if offset != u32::MAX => offset,
            _ => continue,
        };
        let Some((data_type, data)) = type_entry_value(entry_type, offset) else {
            continue;
        };
        let density = u32::from(entry_type.density);
        let density_score = if (120..=640).contains(&density) {
            density
        } else {
            0
        };
        // Highest density wins; PNG edges out an equal-density WebP (both
        // decode, PNG stays lossless). XML entries are skipped below.
        let png_rank = if data_type == 0x03 {
            u32::from(
                arsc.values
                    .strings
                    .get(usize::try_from(data).ok()?)
                    .is_some_and(|path| path.to_ascii_lowercase().ends_with(".png")),
            )
        } else {
            0
        };
        if best
            .as_ref()
            .is_none_or(|prev| (density_score, png_rank) > (prev.1, prev.0))
        {
            best = Some((png_rank, density_score, data_type, data));
        }
    }
    let (_, _, data_type, data) = best?;
    match data_type {
        // STRING.
        0x03 => arsc
            .values
            .strings
            .get(usize::try_from(data).ok()?)
            .map(|path| ResolvedValue::File(path.clone())),
        // REFERENCE: follow the alias.
        0x01 if data != resource_id => arsc_resolve(arsc, data),
        // INT_COLOR_* family.
        0x1C..=0x1F => Some(ResolvedValue::Color(data)),
        _ => None,
    }
}

// ---------------------------------------------------------------- decode

fn decode_icon(bytes: &[u8]) -> Option<image::RgbaImage> {
    image::load_from_memory(bytes)
        .ok()
        .map(|image| image.to_rgba8())
}

fn scaled_dynamic(image: image::RgbaImage) -> Option<ApplicationIconData> {
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return None;
    }
    let long_side = width.max(height);
    let (target_w, target_h) = if long_side > MAX_ICON_DIMENSION {
        let shrink = |side: u32| {
            u32::try_from(
                (u64::from(side) * u64::from(MAX_ICON_DIMENSION)).max(1) / u64::from(long_side),
            )
            .unwrap_or(1)
            .max(1)
        };
        (shrink(width), shrink(height))
    } else {
        (width, height)
    };
    let resized = if (target_w, target_h) == (width, height) {
        image
    } else {
        image::imageops::resize(
            &image,
            target_w,
            target_h,
            image::imageops::FilterType::Triangle,
        )
    };
    Some(ApplicationIconData {
        width: resized.width(),
        height: resized.height(),
        rgba: resized.into_raw(),
    })
}

/// Stacks adaptive-icon layers: background first, foreground over it.
fn compose_layers(layers: Vec<Option<image::RgbaImage>>) -> Option<image::RgbaImage> {
    let mut canvas: Option<image::RgbaImage> = None;
    for layer in layers.into_iter().flatten() {
        canvas = Some(match canvas {
            None => layer,
            Some(base) => {
                let side = base
                    .width()
                    .max(base.height())
                    .max(layer.width())
                    .max(layer.height());
                let mut canvas = image::imageops::resize(
                    &base,
                    side,
                    side,
                    image::imageops::FilterType::Triangle,
                );
                let layer = image::imageops::resize(
                    &layer,
                    side,
                    side,
                    image::imageops::FilterType::Triangle,
                );
                image::imageops::overlay(&mut canvas, &layer, 0, 0);
                canvas
            }
        });
    }
    canvas
}

fn color_layer(argb: u32) -> image::RgbaImage {
    image::RgbaImage::from_pixel(
        MAX_ICON_DIMENSION,
        MAX_ICON_DIMENSION,
        image::Rgba([
            ((argb >> 16) & 0xFF) as u8,
            ((argb >> 8) & 0xFF) as u8,
            (argb & 0xFF) as u8,
            ((argb >> 24) & 0xFF) as u8,
        ]),
    )
}

// ------------------------------------------------------------- pipeline

async fn fetch_entry_bytes(
    executable: &Path,
    serial: &DeviceSerial,
    apk: &str,
    entries: &[ZipEntry],
    name: &str,
    timeout: Duration,
) -> Option<Vec<u8>> {
    let entry = entries.iter().find(|entry| entry.name == name)?;
    if entry.compressed_size > ENTRY_LIMIT {
        return None;
    }
    // Read the local header plus its data in one go; the local extra field
    // may differ from the central one, hence the slack.
    let slack = 4096u64;
    let name_bytes = u64::try_from(entry.name.len()).unwrap_or(0);
    let buffer = read_range(
        executable,
        serial,
        apk,
        entry.local_offset,
        30 + name_bytes + slack + entry.compressed_size,
        timeout,
    )
    .await
    .ok()?;
    entry_data_from_local(&buffer, entry)
}

async fn resolved_layer(
    executable: &Path,
    serial: &DeviceSerial,
    apk: &str,
    entries: &[ZipEntry],
    value: &ResolvedValue,
    timeout: Duration,
) -> Option<image::RgbaImage> {
    match value {
        ResolvedValue::Color(argb) => Some(color_layer(*argb)),
        ResolvedValue::File(path) => {
            if path.to_ascii_lowercase().ends_with(".xml") {
                return None;
            }
            let bytes = fetch_entry_bytes(executable, serial, apk, entries, path, timeout).await?;
            decode_icon(&bytes)
        }
    }
}

/// Resolves an adaptive-icon XML file into a composed layer image.
async fn adaptive_layers(
    executable: &Path,
    serial: &DeviceSerial,
    apk: &str,
    entries: &[ZipEntry],
    arsc: &Arsc,
    xml_path: &str,
    timeout: Duration,
) -> Option<image::RgbaImage> {
    let xml = fetch_entry_bytes(executable, serial, apk, entries, xml_path, timeout).await?;
    let elements = parse_axml(&xml)?;
    let mut layers = Vec::new();
    for element_name in ["background", "foreground"] {
        let resolved = axml_reference(&elements, element_name, "drawable")
            .and_then(|reference| arsc_resolve(arsc, reference));
        if let Some(value) = resolved {
            layers.push(resolved_layer(executable, serial, apk, entries, &value, timeout).await);
        }
    }
    compose_layers(layers)
}

/// Extracts the launcher icon, or `None` when the APK does not expose one
/// this module understands. All device/parse failures degrade to `None`.
pub(crate) async fn extract_application_icon(
    executable: &Path,
    serial: &DeviceSerial,
    package: &PackageName,
    timeout: Duration,
) -> Result<Option<ApplicationIconData>, BridgeError> {
    Ok(try_extract(executable, serial, package, timeout).await)
}

/// ZIP directory plus parsed `resources.arsc` for one APK.
struct ApkContext {
    apk: String,
    entries: Vec<ZipEntry>,
    arsc: Arsc,
}

async fn load_context(
    executable: &Path,
    serial: &DeviceSerial,
    apk: &str,
    size: u64,
    timeout: Duration,
) -> Option<ApkContext> {
    if size < 64 {
        return None;
    }

    // ZIP tail: end-of-central-directory plus (usually) the whole directory.
    let tail_len = size.min(TAIL_BYTES);
    let tail_start = size - tail_len;
    let tail = read_range(executable, serial, apk, tail_start, tail_len, timeout)
        .await
        .ok()?;
    let (cd_offset, cd_size) = find_eocd(&tail)?;
    let tail_end = tail_start.saturating_add(u64::try_from(tail.len()).unwrap_or(0));
    let directory = if cd_offset >= tail_start && cd_offset + cd_size <= tail_end {
        let from = usize::try_from(cd_offset - tail_start).ok()?;
        let len = usize::try_from(cd_size).ok()?;
        tail.get(from..from.checked_add(len)?).map(<[u8]>::to_vec)
    } else if cd_size <= CENTRAL_DIR_LIMIT {
        read_range(executable, serial, apk, cd_offset, cd_size, timeout)
            .await
            .ok()
    } else {
        None
    };
    let directory = directory?;
    let entries = parse_central_directory(&directory)?;
    let arsc_bytes =
        fetch_entry_bytes(executable, serial, apk, &entries, "resources.arsc", timeout).await?;
    let arsc = parse_arsc(&arsc_bytes)?;
    Some(ApkContext {
        apk: apk.to_owned(),
        entries,
        arsc,
    })
}

async fn try_extract(
    executable: &Path,
    serial: &DeviceSerial,
    package: &PackageName,
    timeout: Duration,
) -> Option<ApplicationIconData> {
    let (apk, size) = apk_path_and_size(executable, serial, package, timeout).await?;
    let context = load_context(executable, serial, &apk, size, timeout).await?;

    // Manifest -> icon resource id.
    let manifest = fetch_entry_bytes(
        executable,
        serial,
        &context.apk,
        &context.entries,
        "AndroidManifest.xml",
        timeout,
    )
    .await?;
    let elements = parse_axml(&manifest)?;
    let icon_id = axml_reference(&elements, "application", "icon")?;

    // Apps without their own icon reference the framework default
    // (`@android:drawable/sym_def_app_icon`); that id lives in the system's
    // framework-res.apk, not in the app's resource table.
    let framework_ref = context
        .arsc
        .packages
        .first()
        .is_none_or(|package| package.id != icon_id >> 24);
    let icon_context = if framework_ref {
        let framework_apk = "/system/framework/framework-res.apk";
        let framework_size = apk_size(executable, serial, framework_apk, timeout).await?;
        load_context(executable, serial, framework_apk, framework_size, timeout).await?
    } else {
        context
    };
    let resolved = arsc_resolve(&icon_context.arsc, icon_id)?;

    let image = match &resolved {
        ResolvedValue::File(path) if path.to_ascii_lowercase().ends_with(".xml") => {
            adaptive_layers(
                executable,
                serial,
                &icon_context.apk,
                &icon_context.entries,
                &icon_context.arsc,
                path,
                timeout,
            )
            .await
        }
        value => {
            resolved_layer(
                executable,
                serial,
                &icon_context.apk,
                &icon_context.entries,
                value,
                timeout,
            )
            .await
        }
    };
    image.and_then(scaled_dynamic)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn push_u8(out: &mut Vec<u8>, value: u8) {
        out.push(value);
    }

    fn push_u16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    // ------------------------------------------------------------ zip

    struct FixtureEntry<'a> {
        name: &'a str,
        data: &'a [u8],
        deflated: bool,
    }

    fn deflate(data: &[u8]) -> Vec<u8> {
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
        encoder.write_all(data).expect("write deflate fixture");
        encoder.finish().expect("finish deflate fixture")
    }

    /// Builds a complete ZIP file (local entries + central dir + EOCD).
    fn build_zip(entries: &[FixtureEntry<'_>]) -> Vec<u8> {
        let mut body = Vec::new();
        let mut central = Vec::new();
        for entry in entries {
            let (method, payload) = if entry.deflated {
                (METHOD_DEFLATED, deflate(entry.data))
            } else {
                (METHOD_STORED, entry.data.to_vec())
            };
            let local_offset = u32::try_from(body.len()).expect("fixture fits u32");
            push_u32(&mut body, LOCAL_MAGIC);
            push_u16(&mut body, 20); // version needed
            push_u16(&mut body, 0); // flags
            push_u16(&mut body, method);
            push_u16(&mut body, 0); // time
            push_u16(&mut body, 0); // date
            push_u32(&mut body, 0); // crc
            push_u32(&mut body, u32::try_from(payload.len()).expect("fits"));
            push_u32(&mut body, u32::try_from(entry.data.len()).expect("fits"));
            push_u16(&mut body, u16::try_from(entry.name.len()).expect("fits"));
            push_u16(&mut body, 0); // extra
            body.extend_from_slice(entry.name.as_bytes());
            body.extend_from_slice(&payload);

            push_u32(&mut central, CENTRAL_MAGIC);
            push_u16(&mut central, 20); // version made by
            push_u16(&mut central, 20); // version needed
            push_u16(&mut central, 0); // flags
            push_u16(&mut central, method);
            push_u16(&mut central, 0); // time
            push_u16(&mut central, 0); // date
            push_u32(&mut central, 0); // crc
            push_u32(&mut central, u32::try_from(payload.len()).expect("fits"));
            push_u32(&mut central, u32::try_from(entry.data.len()).expect("fits"));
            push_u16(&mut central, u16::try_from(entry.name.len()).expect("fits"));
            push_u16(&mut central, 0); // extra
            push_u16(&mut central, 0); // comment
            push_u16(&mut central, 0); // disk
            push_u16(&mut central, 0); // internal attrs
            push_u32(&mut central, 0); // external attrs
            push_u32(&mut central, local_offset);
            central.extend_from_slice(entry.name.as_bytes());
        }
        let cd_offset = u32::try_from(body.len()).expect("fits");
        let cd_size = u32::try_from(central.len()).expect("fits");
        let mut out = body;
        out.extend_from_slice(&central);
        push_u32(&mut out, EOCD_MAGIC);
        push_u16(&mut out, 0); // disk
        push_u16(&mut out, 0); // cd disk
        push_u16(&mut out, u16::try_from(entries.len()).expect("fits"));
        push_u16(&mut out, u16::try_from(entries.len()).expect("fits"));
        push_u32(&mut out, cd_size);
        push_u32(&mut out, cd_offset);
        push_u16(&mut out, 0); // comment length
        out
    }

    #[test]
    fn locates_eocd_and_parses_central_directory() {
        let manifest = b"binary manifest".to_vec();
        let arsc = b"binary resources".to_vec();
        let icon = vec![0x89, b'P', b'N', b'G', 1, 2, 3];
        let file = build_zip(&[
            FixtureEntry {
                name: "AndroidManifest.xml",
                data: &manifest,
                deflated: true,
            },
            FixtureEntry {
                name: "resources.arsc",
                data: &arsc,
                deflated: false,
            },
            FixtureEntry {
                name: "res/mipmap/ic.png",
                data: &icon,
                deflated: false,
            },
        ]);

        let Some((cd_offset, cd_size)) = find_eocd(&file) else {
            panic!("eocd must be found");
        };
        let from = usize::try_from(cd_offset).expect("fits");
        let to = usize::try_from(cd_offset + cd_size).expect("fits");
        let entries = parse_central_directory(&file[from..to]).expect("parse directory");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "AndroidManifest.xml");
        assert_eq!(entries[0].method, METHOD_DEFLATED);
        assert_eq!(entries[2].name, "res/mipmap/ic.png");
        assert_eq!(entries[2].method, METHOD_STORED);

        // Local-header slicing recovers the original bytes either way.
        for (entry, expected) in entries.iter().zip([&manifest, &arsc, &icon]) {
            let at = usize::try_from(entry.local_offset).expect("fits");
            let buffer = &file[at..];
            assert_eq!(
                entry_data_from_local(buffer, entry).expect("entry data"),
                *expected
            );
        }
    }

    #[test]
    fn rejects_corrupt_local_headers() {
        let data = b"payload".to_vec();
        let file = build_zip(&[FixtureEntry {
            name: "a.bin",
            data: &data,
            deflated: false,
        }]);
        let Some((cd_offset, cd_size)) = find_eocd(&file) else {
            panic!("eocd must be found");
        };
        let from = usize::try_from(cd_offset).expect("fits");
        let to = usize::try_from(cd_offset + cd_size).expect("fits");
        let entries = parse_central_directory(&file[from..to]).expect("parse");
        assert!(entry_data_from_local(&[], &entries[0]).is_none());
        assert!(entry_data_from_local(&[0_u8; 64], &entries[0]).is_none());
    }

    // ------------------------------------------------------ string pool

    fn build_string_pool_utf16(strings: &[&str]) -> Vec<u8> {
        let mut chunk = Vec::new();
        push_u16(&mut chunk, 0x0001);
        push_u16(&mut chunk, 28);
        let size_at = chunk.len();
        push_u32(&mut chunk, 0);
        push_u32(&mut chunk, u32::try_from(strings.len()).expect("fits"));
        push_u32(&mut chunk, 0); // style count
        push_u32(&mut chunk, 0); // flags: UTF-16
        push_u32(
            &mut chunk,
            28 + u32::try_from(strings.len()).expect("fits") * 4,
        );
        push_u32(&mut chunk, 0); // styles start
        let mut offsets = Vec::new();
        let mut blob = Vec::new();
        for string in strings {
            offsets.push(u32::try_from(blob.len()).expect("fits"));
            push_u16(
                &mut blob,
                u16::try_from(string.chars().count()).expect("fits"),
            );
            for unit in string.encode_utf16() {
                push_u16(&mut blob, unit);
            }
            push_u16(&mut blob, 0);
        }
        for offset in offsets {
            push_u32(&mut chunk, offset);
        }
        chunk.extend_from_slice(&blob);
        let size = u32::try_from(chunk.len()).expect("fits");
        chunk[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
        chunk
    }

    #[test]
    fn string_pool_reads_utf16_entries() {
        let strings = ["", "icon", "res/mipmap-xxxhdpi-v4/ic_launcher.png"];
        let chunk = build_string_pool_utf16(&strings);
        let pool = parse_string_pool(&chunk).expect("parses");
        assert!(pool.strings.iter().eq(strings.iter().copied()));
    }

    // ------------------------------------------------------------- axml

    fn build_start_element(
        strings: &[&str],
        name: &str,
        attributes: &[(&str, u8, u32)],
    ) -> Vec<u8> {
        let name_index = u32::try_from(
            strings
                .iter()
                .position(|candidate| *candidate == name)
                .expect("name in pool"),
        )
        .expect("fits");
        let mut chunk = Vec::new();
        push_u16(&mut chunk, 0x0102);
        push_u16(&mut chunk, 16); // header size
        let size_at = chunk.len();
        push_u32(&mut chunk, 0);
        push_u32(&mut chunk, 1); // line number
        push_u32(&mut chunk, u32::MAX); // comment
        push_u32(&mut chunk, u32::MAX); // ns
        push_u32(&mut chunk, name_index);
        push_u16(&mut chunk, 20); // attribute start
        push_u16(&mut chunk, 20); // attribute size
        push_u16(&mut chunk, u16::try_from(attributes.len()).expect("fits"));
        push_u16(&mut chunk, 0);
        push_u16(&mut chunk, 0);
        push_u16(&mut chunk, 0);
        for (attribute_name, data_type, data) in attributes {
            let index = u32::try_from(
                strings
                    .iter()
                    .position(|candidate| *candidate == *attribute_name)
                    .expect("attribute in pool"),
            )
            .expect("fits");
            push_u32(&mut chunk, u32::MAX); // ns
            push_u32(&mut chunk, index);
            push_u32(&mut chunk, u32::MAX); // raw value
            push_u16(&mut chunk, 8); // typed value size
            push_u8(&mut chunk, 0); // res0
            push_u8(&mut chunk, *data_type);
            push_u32(&mut chunk, *data);
        }
        while chunk.len() % 4 != 0 {
            push_u8(&mut chunk, 0);
        }
        let size = u32::try_from(chunk.len()).expect("fits");
        chunk[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
        chunk
    }

    fn build_axml(strings: &[&str], elements: Vec<Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        push_u16(&mut out, 0x0003);
        push_u16(&mut out, 8);
        let size_at = 4;
        push_u32(&mut out, 0);
        out.extend_from_slice(&build_string_pool_utf16(strings));
        for element in elements {
            out.extend_from_slice(&element);
        }
        let size = u32::try_from(out.len()).expect("fits");
        out[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
        out
    }

    #[test]
    fn axml_exposes_element_and_reference_attributes() {
        let strings = [
            "manifest",
            "application",
            "icon",
            "adaptive-icon",
            "foreground",
            "drawable",
        ];
        let application =
            build_start_element(&strings, "application", &[("icon", 0x01, 0x7F01_0001)]);
        let foreground =
            build_start_element(&strings, "foreground", &[("drawable", 0x01, 0x7F02_0003)]);
        let data = build_axml(&strings, vec![application, foreground]);

        let elements = parse_axml(&data).expect("parse axml");
        assert_eq!(elements.len(), 2);
        assert_eq!(
            axml_reference(&elements, "application", "icon"),
            Some(0x7F01_0001)
        );
        assert_eq!(
            axml_reference(&elements, "foreground", "drawable"),
            Some(0x7F02_0003)
        );
        assert_eq!(axml_reference(&elements, "missing", "icon"), None);
        assert_eq!(axml_reference(&elements, "application", "drawable"), None);
    }

    #[test]
    fn axml_rejects_non_xml_buffers() {
        assert!(parse_axml(b"garbage").is_none());
        assert!(parse_axml(&[]).is_none());
    }

    // ------------------------------------------------------------- arsc

    struct FixtureType {
        id: u8,
        density: u16,
        sparse: bool,
        /// `None` = NO_ENTRY; `(data_type, data)` otherwise.
        entries: Vec<Option<(u8, u32)>>,
    }

    fn build_config(density: u16) -> Vec<u8> {
        let mut config = vec![0_u8; 28];
        config[0..4].copy_from_slice(&28_u32.to_le_bytes());
        config[14..16].copy_from_slice(&density.to_le_bytes());
        config
    }

    fn build_type_chunk(entry_type: &FixtureType) -> Vec<u8> {
        let present = entry_type
            .entries
            .iter()
            .filter(|slot| slot.is_some())
            .count();
        let config = build_config(entry_type.density);
        let offsets_size = if entry_type.sparse {
            present * 4
        } else {
            entry_type.entries.len() * 4
        };
        let header_size = 20 + config.len();
        let entries_start = header_size + offsets_size;

        let mut chunk = Vec::new();
        push_u16(&mut chunk, 0x0201);
        push_u16(&mut chunk, u16::try_from(header_size).expect("fits"));
        let size_at = chunk.len();
        push_u32(&mut chunk, 0);
        push_u8(&mut chunk, entry_type.id);
        push_u8(&mut chunk, u8::from(entry_type.sparse));
        push_u16(&mut chunk, 0);
        push_u32(
            &mut chunk,
            u32::try_from(entry_type.entries.len()).expect("fits"),
        );
        push_u32(&mut chunk, u32::try_from(entries_start).expect("fits"));
        chunk.extend_from_slice(&config);

        let mut body = Vec::new();
        let mut offsets = Vec::new();
        for (index, slot) in entry_type.entries.iter().enumerate() {
            let Some((data_type, data)) = slot else {
                if !entry_type.sparse {
                    push_u32(&mut offsets, u32::MAX);
                }
                continue;
            };
            if entry_type.sparse {
                push_u16(&mut offsets, u16::try_from(index).expect("fits"));
                push_u16(
                    &mut offsets,
                    u16::try_from(body.len() / 4).expect("aligned offset"),
                );
            } else {
                push_u32(&mut offsets, u32::try_from(body.len()).expect("fits"));
            }
            push_u16(&mut body, 8); // entry size
            push_u16(&mut body, 0); // flags: simple
            push_u32(&mut body, u32::try_from(index).expect("fits")); // key
            push_u16(&mut body, 8); // Res_value size
            push_u8(&mut body, 0); // res0
            push_u8(&mut body, *data_type);
            push_u32(&mut body, *data);
        }
        chunk.extend_from_slice(&offsets);
        chunk.extend_from_slice(&body);
        let size = u32::try_from(chunk.len()).expect("fits");
        chunk[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
        chunk
    }

    fn build_type_spec(id: u8) -> Vec<u8> {
        let mut chunk = Vec::new();
        push_u16(&mut chunk, 0x0202);
        push_u16(&mut chunk, 12); // header: type/size/id/res0/res2
        let size_at = chunk.len();
        push_u32(&mut chunk, 0);
        push_u8(&mut chunk, id);
        push_u8(&mut chunk, 0);
        push_u16(&mut chunk, 0);
        push_u32(&mut chunk, 0); // one entry-mask word
        let size = u32::try_from(chunk.len()).expect("fits");
        chunk[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
        chunk
    }

    fn build_arsc(values: &[&str], package_id: u32, types: &[FixtureType]) -> Vec<u8> {
        let mut out = Vec::new();
        push_u16(&mut out, 0x0002);
        push_u16(&mut out, 12);
        let size_at = 4;
        push_u32(&mut out, 0);
        push_u32(&mut out, 1); // package count
        out.extend_from_slice(&build_string_pool_utf16(values));

        let mut package = Vec::new();
        push_u16(&mut package, 0x0200);
        push_u16(&mut package, 288);
        let package_size_at = package.len();
        push_u32(&mut package, 0);
        push_u32(&mut package, package_id);
        package.extend(std::iter::repeat_n(0_u8, 256)); // name
        push_u32(&mut package, 0); // type strings
        push_u32(&mut package, 0); // last public type
        push_u32(&mut package, 0); // key strings
        push_u32(&mut package, 0); // last public key
        push_u32(&mut package, 0); // type id offset
        for entry_type in types {
            package.extend_from_slice(&build_type_spec(entry_type.id));
            package.extend_from_slice(&build_type_chunk(entry_type));
        }
        let package_size = u32::try_from(package.len()).expect("fits");
        package[package_size_at..package_size_at + 4].copy_from_slice(&package_size.to_le_bytes());
        out.extend_from_slice(&package);

        let size = u32::try_from(out.len()).expect("fits");
        out[size_at..size_at + 4].copy_from_slice(&size.to_le_bytes());
        out
    }

    fn string_value(value: usize) -> (u8, u32) {
        (0x03, u32::try_from(value).expect("fits"))
    }

    #[test]
    fn arsc_resolves_the_densest_string_entry() {
        let values = ["res/mipmap-mdpi/ic.png", "res/mipmap-xxxhdpi/ic.png"];
        let data = build_arsc(
            &values,
            0x7F,
            &[
                FixtureType {
                    id: 1,
                    density: 160,
                    sparse: false,
                    entries: vec![Some(string_value(0))],
                },
                FixtureType {
                    id: 1,
                    density: 640,
                    sparse: false,
                    entries: vec![Some(string_value(1))],
                },
            ],
        );
        let arsc = parse_arsc(&data).expect("parse arsc");
        match arsc_resolve(&arsc, 0x7F01_0000).expect("resolves") {
            ResolvedValue::File(path) => assert_eq!(path, "res/mipmap-xxxhdpi/ic.png"),
            ResolvedValue::Color(_) => panic!("expected a file path"),
        }
    }

    #[test]
    fn arsc_prefers_the_densest_entry_and_png_breaks_ties() {
        let values = ["res/mipmap-anydpi/ic.xml", "res/mipmap/ic.webp"];
        let data = build_arsc(
            &values,
            0x7F,
            &[
                FixtureType {
                    id: 1,
                    density: 640,
                    sparse: false,
                    entries: vec![Some(string_value(1))],
                },
                FixtureType {
                    id: 1,
                    density: 160,
                    sparse: false,
                    entries: vec![Some(string_value(0))],
                },
            ],
        );
        let arsc = parse_arsc(&data).expect("parse arsc");
        match arsc_resolve(&arsc, 0x7F01_0000).expect("resolves") {
            ResolvedValue::File(path) => assert_eq!(path, "res/mipmap/ic.webp"),
            ResolvedValue::Color(_) => panic!("expected a file path"),
        }

        // A denser WebP beats a low-density PNG: density ranks first.
        let values = ["res/mipmap/ic.png", "res/mipmap-xxxhdpi/ic.webp"];
        let data = build_arsc(
            &values,
            0x7F,
            &[
                FixtureType {
                    id: 1,
                    density: 160,
                    sparse: false,
                    entries: vec![Some(string_value(0))],
                },
                FixtureType {
                    id: 1,
                    density: 640,
                    sparse: false,
                    entries: vec![Some(string_value(1))],
                },
            ],
        );
        let arsc = parse_arsc(&data).expect("parse arsc");
        match arsc_resolve(&arsc, 0x7F01_0000).expect("resolves") {
            ResolvedValue::File(path) => assert_eq!(path, "res/mipmap-xxxhdpi/ic.webp"),
            ResolvedValue::Color(_) => panic!("expected a file path"),
        }

        // Equal density: the lossless PNG wins.
        let values = ["res/mipmap-xhdpi/ic.png", "res/mipmap-xhdpi/ic.webp"];
        let data = build_arsc(
            &values,
            0x7F,
            &[
                FixtureType {
                    id: 1,
                    density: 480,
                    sparse: false,
                    entries: vec![Some(string_value(0))],
                },
                FixtureType {
                    id: 1,
                    density: 480,
                    sparse: false,
                    entries: vec![Some(string_value(1))],
                },
            ],
        );
        let arsc = parse_arsc(&data).expect("parse arsc");
        match arsc_resolve(&arsc, 0x7F01_0000).expect("resolves") {
            ResolvedValue::File(path) => assert_eq!(path, "res/mipmap-xhdpi/ic.png"),
            ResolvedValue::Color(_) => panic!("expected a file path"),
        }
    }

    #[test]
    fn arsc_follows_reference_aliases_and_skips_missing_entries() {
        let values = ["res/drawable/ic.png"];
        let data = build_arsc(
            &values,
            0x7F,
            &[
                // drawable: entry 0 aliases mipmap entry 0; entry 1 absent.
                FixtureType {
                    id: 2,
                    density: 480,
                    sparse: false,
                    entries: vec![Some((0x01, 0x7F03_0000))],
                },
                FixtureType {
                    id: 3,
                    density: 480,
                    sparse: false,
                    entries: vec![Some(string_value(0)), None],
                },
            ],
        );
        let arsc = parse_arsc(&data).expect("parse arsc");
        match arsc_resolve(&arsc, 0x7F02_0000).expect("alias resolves") {
            ResolvedValue::File(path) => assert_eq!(path, "res/drawable/ic.png"),
            ResolvedValue::Color(_) => panic!("expected a file path"),
        }
        assert!(arsc_resolve(&arsc, 0x7F03_0001).is_none());
        assert!(arsc_resolve(&arsc, 0x0101_0000).is_none());
    }

    #[test]
    fn arsc_resolves_sparse_and_color_entries() {
        let values = ["res/mipmap/ic.png"];
        let data = build_arsc(
            &values,
            0x7F,
            &[
                // Sparse mipmap table: only entry 1 is present.
                FixtureType {
                    id: 1,
                    density: 480,
                    sparse: true,
                    entries: vec![None, Some(string_value(0))],
                },
                // Color resource usable as an adaptive background.
                FixtureType {
                    id: 2,
                    density: 0,
                    sparse: false,
                    entries: vec![Some((0x1C, 0xFFFF_FFFF))],
                },
            ],
        );
        let arsc = parse_arsc(&data).expect("parse arsc");
        assert!(matches!(
            arsc_resolve(&arsc, 0x7F01_0001),
            Some(ResolvedValue::File(path)) if path == "res/mipmap/ic.png"
        ));
        assert!(matches!(
            arsc_resolve(&arsc, 0x7F02_0000),
            Some(ResolvedValue::Color(0xFFFF_FFFF))
        ));
        assert!(arsc_resolve(&arsc, 0x7F01_0000).is_none());
    }

    #[test]
    fn arsc_rejects_non_table_buffers() {
        assert!(parse_arsc(b"garbage").is_none());
        assert!(parse_arsc(&[]).is_none());
    }

    // ------------------------------------------------------------ decode

    fn png_fixture(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([9, 8, 7, 255]));
        let mut buffer = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut buffer, image::ImageFormat::Png)
            .expect("encode png fixture");
        buffer.into_inner()
    }

    #[test]
    fn decodes_and_downscales_to_the_grid_size() {
        let icon = decode_icon(&png_fixture(300, 150)).expect("decodes");
        let data = scaled_dynamic(icon).expect("scales");
        assert_eq!((data.width, data.height), (128, 64));
        assert_eq!(
            data.rgba.len(),
            usize::try_from(data.width * data.height).expect("fits") * 4
        );
    }

    #[test]
    fn keeps_icons_that_already_fit() {
        let icon = decode_icon(&png_fixture(96, 96)).expect("decodes");
        let data = scaled_dynamic(icon).expect("kept");
        assert_eq!((data.width, data.height), (96, 96));
        assert!(decode_icon(b"not an image").is_none());
    }

    #[test]
    fn parses_combined_path_and_size_output() {
        let output = "path:/data/app/~~abc==/com.example-eZs==/base.apk\n69174389\n";
        let (path, size) = parse_path_and_size(output).expect("parses");
        assert_eq!(path, "/data/app/~~abc==/com.example-eZs==/base.apk");
        assert_eq!(size, 69_174_389);
        // wc fallback prints only the number; split apks fall back to the
        // first listed path.
        assert!(parse_path_and_size("path:/data/app/a/split_config.apk\n123\n").is_some());
        assert!(parse_path_and_size("path:\n123\n").is_none());
        assert!(parse_path_and_size("123\n").is_none());
    }

    #[test]
    fn compose_layers_stacks_background_under_foreground() {
        let background = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
        let foreground = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 255, 255]));
        let composed = compose_layers(vec![Some(background), Some(foreground)]).expect("composes");
        assert_eq!(composed.get_pixel(0, 0), &image::Rgba([0, 0, 255, 255]));
        assert!(compose_layers(vec![None, None]).is_none());
    }

    /// Manual probe against a live device — set `BRIDGESCOPE_ICON_PROBE` to
    /// the serial, `BRIDGESCOPE_ICON_PACKAGE` to a package (optionally
    /// `BRIDGESCOPE_ADB` to the executable) and run
    /// `cargo test -p bridgescope-adb -- --ignored probe_real_device_icon`.
    /// Decodes are dumped to the temp dir for inspection.
    #[tokio::test]
    #[ignore = "needs a live device; drive it with BRIDGESCOPE_ICON_PROBE"]
    async fn probe_real_device_icon() {
        let Some(serial) = std::env::var_os("BRIDGESCOPE_ICON_PROBE") else {
            return;
        };
        let package = std::env::var("BRIDGESCOPE_ICON_PACKAGE").expect("package env set");
        let executable = std::env::var_os("BRIDGESCOPE_ADB")
            .map_or_else(|| std::path::PathBuf::from("adb"), std::path::PathBuf::from);
        let serial = bridgescope_domain::DeviceSerial::new(serial.to_string_lossy().into_owned())
            .expect("valid serial");
        let package = bridgescope_domain::PackageName::new(&package).expect("valid package");

        let Some(icon) = super::extract_application_icon(
            &executable,
            &serial,
            &package,
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("extraction succeeds") else {
            eprintln!("probe: no extractable icon for {package}");
            return;
        };
        eprintln!("probe: icon {}x{}", icon.width, icon.height);
        let image = image::RgbaImage::from_raw(icon.width, icon.height, icon.rgba)
            .expect("dimensions match pixels");
        let path = std::env::temp_dir().join("bridgescope-icon-probe.png");
        image.save(&path).expect("saves");
        eprintln!("probe: written to {}", path.display());
    }
}
