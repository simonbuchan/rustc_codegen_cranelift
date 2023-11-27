use std::ffi::CStr;

use object::{Object, ObjectSymbol};

mod coff;

use crate::dll_import_lib::coff::ImportDescriptorValues;
pub(crate) use coff::{Import, ImportNameType, ImportType, Machine};

pub(crate) struct ImportLibraryBuilder {
    dll_name: String,
    machine: Machine,
    members: Vec<ar_archive_writer::NewArchiveMember<'static>>,
}

impl ImportLibraryBuilder {
    pub(crate) fn new(dll_name: &str, machine: Machine) -> coff::Result<Self> {
        let values = ImportDescriptorValues::new(dll_name.to_string(), machine);
        let members = vec![
            coff_member(dll_name, coff::generate_import_descriptor(&values)?),
            coff_member(
                dll_name,
                coff::generate_null_thunk_data(machine, &values.null_thunk_data_symbol)?,
            ),
            coff_member(dll_name, coff::generate_null_import_descriptor(machine)?),
        ];
        Ok(Self { dll_name: values.dll_name, machine, members })
    }

    pub(crate) fn add_import(&mut self, import: Import) -> coff::Result<()> {
        self.members.push(import_member(&self.dll_name, self.machine, &import)?);
        Ok(())
    }

    pub(crate) fn write<W>(&self, w: &mut W) -> std::io::Result<()>
    where
        W: ?Sized + std::io::Write + std::io::Seek,
    {
        let mut w = std::io::BufWriter::new(w);
        let write_symtab = true;
        let deterministic = true;
        let thin = false;
        ar_archive_writer::write_archive_to_stream(
            &mut w,
            &self.members,
            write_symtab,
            ar_archive_writer::ArchiveKind::Gnu,
            deterministic,
            thin,
        )?;
        // must flush before drop to ensure any final IO errors are reported.
        std::io::Write::flush(&mut w)?;
        Ok(())
    }
}

fn coff_member(dll_name: &str, buf: Vec<u8>) -> ar_archive_writer::NewArchiveMember<'static> {
    ar_archive_writer::NewArchiveMember {
        member_name: dll_name.to_string(),
        buf: Box::new(buf),
        get_symbols: coff_get_symbols,
        mtime: 0,
        uid: 0,
        gid: 0,
        perms: 0,
    }
}

fn import_member(
    dll_name: &str,
    machine: Machine,
    import: &Import,
) -> coff::Result<ar_archive_writer::NewArchiveMember<'static>> {
    Ok(ar_archive_writer::NewArchiveMember {
        member_name: dll_name.to_string(),
        buf: Box::new(coff::write_short_import(dll_name, machine, import)?),
        get_symbols: short_import_get_symbols,
        mtime: 0,
        uid: 0,
        gid: 0,
        perms: 0,
    })
}

pub(crate) fn get_symbols(
    buf: &[u8],
    f: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<bool> {
    // Try to first parse as a COFF "short" import object first, which the get_object_symbols from
    // rustc doesn't understand.
    if short_import_get_symbols(buf, f)? {
        return Ok(true);
    }
    // If that fails, try to parse as a COFF object file.
    if coff_get_symbols(buf, f)? {
        return Ok(true);
    }
    // Nope, not a COFF file.
    Ok(false)
}

fn short_import_get_symbols(
    buf: &[u8],
    f: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<bool> {
    // This doesn't use `object::pe::ImportObjectHeader` as that asserts 4-byte alignment without
    // the `unaligned` feature enabled, which it currently isn't for this repo, and we don't need
    // to check much.
    const NAME_OFFSET: usize = std::mem::size_of::<object::pe::ImportObjectHeader>();
    if buf.len() <= NAME_OFFSET {
        return Ok(false);
    }
    let sig1 = u16::from_le_bytes([buf[0], buf[1]]);
    let sig2 = u16::from_le_bytes([buf[2], buf[3]]);
    if sig1 != object::pe::IMAGE_FILE_MACHINE_UNKNOWN || sig2 != object::pe::IMPORT_OBJECT_HDR_SIG2
    {
        return Ok(false);
    }

    let name = CStr::from_bytes_until_nul(&buf[NAME_OFFSET..])
        .map_err(|_| std::io::Error::other("short import name is missing nul byte"))?;
    f(name.to_bytes())?;
    // This is needed to link to MSVC-compiled DLLs, which use __imp_ prefix unconditionally.
    // `format!("__imp_{name}")` but avoids going through UTF-8.
    let mut imp_name = Vec::new();
    imp_name.extend_from_slice(b"__imp_");
    imp_name.extend_from_slice(name.to_bytes());
    f(&imp_name)?;
    Ok(true)
}

fn coff_get_symbols(
    buf: &[u8],
    f: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<bool> {
    type NtCoffFile<'data> =
        object::read::coff::CoffFile<'data, &'data [u8], object::pe::ImageFileHeader>;
    let Ok(file) = NtCoffFile::parse(buf) else {
        // Not a COFF file.
        return Ok(false);
    };
    for symbol in file.symbols() {
        if symbol.is_definition() {
            let name = symbol.name_bytes().map_err(std::io::Error::other)?;
            f(name)?;
        }
    }
    Ok(true)
}
