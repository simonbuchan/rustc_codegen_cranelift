use std::fs;
use std::path::{Path, PathBuf};

use rustc_codegen_ssa::back::archive::{
    get_native_object_symbols, ArArchiveBuilder, ArchiveBuilder, ArchiveBuilderBuilder,
};
use rustc_session::Session;

pub(crate) struct ArArchiveBuilderBuilder;

impl ArchiveBuilderBuilder for ArArchiveBuilderBuilder {
    fn new_archive_builder<'a>(&self, sess: &'a Session) -> Box<dyn ArchiveBuilder<'a> + 'a> {
        if sess.target.arch != "x86_64" || !sess.target.is_like_msvc {
            Box::new(ArArchiveBuilder::new(sess, get_native_object_symbols))
        } else {
            Box::new(ArArchiveBuilder::new(sess, crate::dll_import_lib::get_symbols))
        }
    }

    fn create_dll_import_lib(
        &self,
        sess: &Session,
        lib_name: &str,
        dll_imports: &[rustc_session::cstore::DllImport],
        tmpdir: &Path,
        _is_direct_dependency: bool,
    ) -> PathBuf {
        if sess.target.arch != "x86_64" || !sess.target.is_like_msvc {
            sess.span_fatal(
                dll_imports.iter().map(|import| import.span).collect::<Vec<_>>(),
                "cranelift codegen currently only supports raw_dylib on x86_64 msvc targets.",
            )
        }

        let mut builder = match crate::dll_import_lib::ImportLibraryBuilder::new(
            lib_name,
            crate::dll_import_lib::Machine::X86_64,
        ) {
            Ok(import_lib) => import_lib,
            Err(error) => {
                sess.fatal(format!(
                    "failed to create import library `{lib_name}`: {error}",
                    lib_name = lib_name,
                ));
            }
        };

        for import in dll_imports {
            match builder.add_import(crate::dll_import_lib::Import {
                symbol_name: import.name.to_string(),
                ordinal_or_hint: import.ordinal(),
                name_type: match import.import_name_type {
                    Some(rustc_session::cstore::PeImportNameType::Ordinal(_)) => {
                        crate::dll_import_lib::ImportNameType::Ordinal
                    }
                    None | Some(rustc_session::cstore::PeImportNameType::Decorated) => {
                        crate::dll_import_lib::ImportNameType::Name
                    }
                    Some(rustc_session::cstore::PeImportNameType::NoPrefix) => {
                        crate::dll_import_lib::ImportNameType::NameNoPrefix
                    }
                    Some(rustc_session::cstore::PeImportNameType::Undecorated) => {
                        crate::dll_import_lib::ImportNameType::NameUndecorate
                    }
                },
                import_type: crate::dll_import_lib::ImportType::Code,
            }) {
                Ok(()) => {}
                Err(error) => {
                    sess.fatal(format!(
                        "failed to add import `{import}` to import library `{lib_name}`: {error}",
                        import = import.name,
                        lib_name = lib_name,
                    ));
                }
            }
        }

        let lib_path = tmpdir.join(format!(
            "{prefix}{lib_name}_import{suffix}",
            prefix = sess.target.staticlib_prefix,
            suffix = sess.target.staticlib_suffix,
        ));

        let mut file = match fs::OpenOptions::new().write(true).create_new(true).open(&lib_path) {
            Ok(file) => file,
            Err(error) => {
                sess.fatal(format!(
                    "failed to create import library file `{path}`: {error}",
                    path = lib_path.display(),
                ));
            }
        };

        // import_lib.write() internally uses BufWriter, so we don't need anything here.
        if let Err(error) = builder.write(&mut file) {
            sess.fatal(format!(
                "failed to write import library `{path}`: {error}",
                path = lib_path.display(),
            ));
        }

        lib_path
    }
}
