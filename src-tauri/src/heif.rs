//! Minimal ISO-BMFF walker to pull the EXIF item out of AVIF/HEIC files.

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let slice = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(slice)
    }

    fn uint(&mut self, n: usize) -> Option<u64> {
        Some(self.take(n)?.iter().fold(0u64, |acc, &b| (acc << 8) | b as u64))
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.uint(1)? as u8)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(self.uint(2)? as u16)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(self.uint(4)? as u32)
    }

    /// Next box as (type, payload).
    fn next_box(&mut self) -> Option<([u8; 4], &'a [u8])> {
        let start = self.pos;
        let size = self.u32()? as u64;
        let kind: [u8; 4] = self.take(4)?.try_into().ok()?;
        let size = match size {
            0 => (self.data.len() - start) as u64,
            1 => self.uint(8)?,
            s => s,
        };
        let header = self.pos - start;
        let payload = self.take((size as usize).checked_sub(header)?)?;
        Some((kind, payload))
    }
}

fn find_box<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut r = Reader::new(data);
    while let Some((k, payload)) = r.next_box() {
        if &k == kind {
            return Some(payload);
        }
    }
    None
}

/// Item id of the first item of type `Exif` in an `iinf` box.
fn exif_item_id(iinf: &[u8]) -> Option<u32> {
    let mut r = Reader::new(iinf);
    let version = r.u8()?;
    r.take(3)?;
    let count = if version == 0 { r.u16()? as u32 } else { r.u32()? };
    for _ in 0..count {
        let (kind, infe) = r.next_box()?;
        if &kind != b"infe" {
            continue;
        }
        let mut e = Reader::new(infe);
        let v = e.u8()?;
        e.take(3)?;
        if v < 2 {
            continue;
        }
        let id = if v == 2 { e.u16()? as u32 } else { e.u32()? };
        e.u16()?; // protection index
        if e.take(4)? == b"Exif" {
            return Some(id);
        }
    }
    None
}

/// File byte ranges of an item, from the `iloc` box (file-offset construction only).
fn item_extents(iloc: &[u8], wanted: u32) -> Option<Vec<(usize, usize)>> {
    let mut r = Reader::new(iloc);
    let version = r.u8()?;
    r.take(3)?;
    let sizes = r.u8()?;
    let (offset_size, length_size) = ((sizes >> 4) as usize, (sizes & 0xF) as usize);
    let sizes = r.u8()?;
    let base_offset_size = (sizes >> 4) as usize;
    let index_size = if version >= 1 { (sizes & 0xF) as usize } else { 0 };
    let count = if version < 2 { r.u16()? as u32 } else { r.u32()? };
    for _ in 0..count {
        let id = if version < 2 { r.u16()? as u32 } else { r.u32()? };
        let method = if version >= 1 { r.u16()? & 0xF } else { 0 };
        r.u16()?; // data reference index
        let base = r.uint(base_offset_size)? as usize;
        let extent_count = r.u16()?;
        let mut extents = Vec::new();
        for _ in 0..extent_count {
            r.uint(index_size)?;
            let offset = r.uint(offset_size)? as usize;
            let length = r.uint(length_size)? as usize;
            extents.push((base + offset, length));
        }
        if id == wanted {
            return (method == 0).then_some(extents);
        }
    }
    None
}

/// The TIFF-structured EXIF payload stored in an AVIF/HEIC file, if any.
pub fn exif_tiff(file: &[u8]) -> Option<Vec<u8>> {
    let meta = find_box(file, b"meta")?;
    let children = meta.get(4..)?; // full box header
    let id = exif_item_id(find_box(children, b"iinf")?)?;
    let mut data = Vec::new();
    for (offset, length) in item_extents(find_box(children, b"iloc")?, id)? {
        data.extend_from_slice(file.get(offset..offset.checked_add(length)?)?);
    }
    // Payload starts with a 4-byte offset to the TIFF header.
    let skip = u32::from_be_bytes(data.get(0..4)?.try_into().ok()?) as usize;
    data.get(4 + skip..).map(<[u8]>::to_vec)
}
