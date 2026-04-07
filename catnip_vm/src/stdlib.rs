// FILE: catnip_vm/src/stdlib.rs
//! Stdlib module tests for PurePipeline.
//!
//! The actual stdlib implementations live in catnip_libs/*/rust/ as native
//! plugins. This file only contains integration tests that verify the modules
//! work correctly when loaded through the plugin discovery system.

#[cfg(test)]
mod tests {
    use crate::loader::PureImportLoader;
    use crate::pipeline::PurePipeline;

    #[test]
    fn test_sys_platform() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('sys')\nsys.platform").unwrap();
        assert!(result.is_native_str());
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert!(!s.is_empty());
    }

    #[test]
    fn test_sys_version() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('sys')\nsys.version").unwrap();
        assert!(result.is_native_str());
    }

    #[test]
    fn test_sys_cpu_count() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('sys')\nsys.cpu_count").unwrap();
        let n = result.as_int().expect("cpu_count should be int");
        assert!(n >= 1);
    }

    #[test]
    fn test_sys_exit() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let err = pipeline.execute("import('sys')\nsys.exit(0)").unwrap_err();
        match err {
            crate::error::VMError::Exit(code) => assert_eq!(code, 0),
            other => panic!("expected Exit(0), got: {}", other),
        }
    }

    #[test]
    fn test_sys_exit_nonzero() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let err = pipeline.execute("import('sys')\nsys.exit(42)").unwrap_err();
        match err {
            crate::error::VMError::Exit(code) => assert_eq!(code, 42),
            other => panic!("expected Exit(42), got: {}", other),
        }
    }

    #[test]
    fn test_io_protocol() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('io')\nio.PROTOCOL").unwrap();
        assert!(result.is_native_str());
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(s, "rust");
    }

    #[test]
    fn test_io_print() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        // io.print returns Nil
        let result = pipeline.execute("import('io')\nio.print(1, 2, 3)").unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn test_io_write() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('io')\nio.write(\"hello\")").unwrap();
        assert!(result.is_nil());
    }

    #[test]
    fn test_sys_argv() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('sys')\nsys.argv").unwrap();
        assert!(result.is_native_list());
    }

    // -- io.open tests --

    #[test]
    fn test_io_open_write_and_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let path_str = path.to_str().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let code = format!(
            "import('io')\nf = io.open(\"{}\", \"w\")\nf.write(\"hello world\")\nf.close()\n\
             g = io.open(\"{}\")\nresult = g.read()\ng.close()\nresult",
            path_str.replace('\\', "\\\\"),
            path_str.replace('\\', "\\\\"),
        );
        let result = pipeline.execute(&code).unwrap();
        assert!(result.is_native_str());
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_io_open_default_mode_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("read_default.txt");
        std::fs::write(&path, "content").unwrap();
        let path_str = path.to_str().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let code = format!(
            "import('io')\nf = io.open(\"{}\")\nf.read()",
            path_str.replace('\\', "\\\\"),
        );
        let result = pipeline.execute(&code).unwrap();
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(s, "content");
    }

    #[test]
    fn test_io_open_file_not_found() {
        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let err = pipeline
            .execute("import('io')\nio.open(\"/nonexistent/path/file.txt\")")
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("FileNotFoundError") || msg.contains("No such file"));
    }

    #[test]
    fn test_io_open_close_twice_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("close2.txt");
        std::fs::write(&path, "x").unwrap();
        let path_str = path.to_str().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let code = format!(
            "import('io')\nf = io.open(\"{}\")\nf.close()\nf.close()\n\"ok\"",
            path_str.replace('\\', "\\\\"),
        );
        let result = pipeline.execute(&code).unwrap();
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(s, "ok");
    }

    #[test]
    fn test_io_open_file_attributes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("attrs.txt");
        std::fs::write(&path, "x").unwrap();
        let path_str = path.to_str().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        // Test .closed attribute
        let code = format!(
            "import('io')\nf = io.open(\"{}\")\nbefore = f.closed\nf.close()\nafter = f.closed\n\
             [before, after]",
            path_str.replace('\\', "\\\\"),
        );
        let result = pipeline.execute(&code).unwrap();
        assert!(result.is_native_list());
        let list = unsafe { result.as_native_list_ref().unwrap() };
        let items = list.as_slice_cloned();
        assert_eq!(items[0].as_bool(), Some(false)); // before close
        assert_eq!(items[1].as_bool(), Some(true)); // after close
    }

    #[test]
    fn test_io_open_readline() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lines.txt");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();
        let path_str = path.to_str().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let code = format!(
            "import('io')\nf = io.open(\"{}\")\na = f.readline()\nb = f.readline()\nf.close()\n[a, b]",
            path_str.replace('\\', "\\\\"),
        );
        let result = pipeline.execute(&code).unwrap();
        assert!(result.is_native_list());
        let list = unsafe { result.as_native_list_ref().unwrap() };
        let items = list.as_slice_cloned();
        let a = unsafe { items[0].as_native_str_ref().unwrap() };
        let b = unsafe { items[1].as_native_str_ref().unwrap() };
        assert_eq!(a, "line1\n");
        assert_eq!(b, "line2\n");
    }

    // -- META tests --

    #[test]
    fn test_meta_setattr_getattr() {
        let mut pipeline = PurePipeline::new().unwrap();
        let result = pipeline.execute("META.x = 42\nMETA.x").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_meta_file_in_imported_module() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("mymod.cat");
        std::fs::write(&mod_path, "result = META.file").unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let result = pipeline.execute("import('mymod')\nmymod.result").unwrap();
        assert!(result.is_native_str());
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert!(
            s.contains("mymod.cat"),
            "META.file should contain the module path, got: {}",
            s
        );
    }

    #[test]
    fn test_meta_exports_filters() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("filtered.cat");
        std::fs::write(
            &mod_path,
            "META.exports = [\"public_fn\"]\npublic_fn = 42\nprivate_fn = 99",
        )
        .unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // public_fn should be exported
        let result = pipeline.execute("import('filtered')\nfiltered.public_fn").unwrap();
        assert_eq!(result.as_int(), Some(42));
    }

    #[test]
    fn test_meta_exports_hides_non_listed() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("hidden.cat");
        std::fs::write(&mod_path, "META.exports = [\"visible\"]\nvisible = 1\nhidden = 2").unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // hidden should not be accessible
        let err = pipeline.execute("import('hidden')\nhidden.hidden").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("attribute") || msg.contains("hidden"),
            "expected attribute error, got: {}",
            msg
        );
    }

    #[test]
    fn test_meta_exports_priority_over_all() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("priority.cat");
        std::fs::write(
            &mod_path,
            "__all__ = [\"from_all\"]\nMETA.exports = [\"from_meta\"]\nfrom_all = 1\nfrom_meta = 2",
        )
        .unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        // META.exports wins over __all__
        let result = pipeline.execute("import('priority')\npriority.from_meta").unwrap();
        assert_eq!(result.as_int(), Some(2));

        // from_all should NOT be exported (META.exports takes priority)
        let mut pipeline2 = PurePipeline::new().unwrap();
        let loader2 = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline2.set_import_loader(loader2);

        let err = pipeline2.execute("import('priority')\npriority.from_all").unwrap_err();
        assert!(err.to_string().contains("attribute") || err.to_string().contains("from_all"));
    }

    #[test]
    fn test_io_open_append() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("append.txt");
        std::fs::write(&path, "first").unwrap();
        let path_str = path.to_str().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let code = format!(
            "import('io')\nf = io.open(\"{p}\", \"a\")\nf.write(\"second\")\nf.close()\n\
             g = io.open(\"{p}\")\nresult = g.read()\ng.close()\nresult",
            p = path_str.replace('\\', "\\\\"),
        );
        let result = pipeline.execute(&code).unwrap();
        let s = unsafe { result.as_native_str_ref().unwrap() };
        assert_eq!(s, "firstsecond");
    }

    // -- Review regression tests --

    #[test]
    fn test_meta_exports_bad_type_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("badtype.cat");
        // META.exports = 42 is not a list/tuple/set
        std::fs::write(&mod_path, "META.exports = 42\nfoo = 1").unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let err = pipeline.execute("import('badtype')").unwrap_err();
        assert!(
            err.to_string().contains("META.exports"),
            "expected TypeError about META.exports, got: {}",
            err
        );
    }

    #[test]
    fn test_meta_exports_missing_name_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("missing.cat");
        // META.exports references a name that doesn't exist
        std::fs::write(&mod_path, "META.exports = [\"nonexistent\"]\nfoo = 1").unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let err = pipeline.execute("import('missing')").unwrap_err();
        assert!(
            err.to_string().contains("nonexistent"),
            "expected error about missing name, got: {}",
            err
        );
    }

    #[test]
    fn test_io_open_mode_non_string_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mode_err.txt");
        std::fs::write(&path, "x").unwrap();
        let path_str = path.to_str().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        let code = format!("import('io')\nio.open(\"{}\", 0)", path_str.replace('\\', "\\\\"),);
        let err = pipeline.execute(&code).unwrap_err();
        assert!(
            err.to_string().contains("mode") && err.to_string().contains("string"),
            "expected TypeError about mode, got: {}",
            err
        );
    }

    #[test]
    fn test_meta_exports_nil_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("nilexports.cat");
        std::fs::write(&mod_path, "META.exports = nil\nfoo = 1").unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let err = pipeline.execute("import('nilexports')").unwrap_err();
        assert!(
            err.to_string().contains("META.exports"),
            "expected error about META.exports type, got: {}",
            err
        );
    }

    #[test]
    fn test_meta_exports_mixed_types_errors() {
        let dir = tempfile::tempdir().unwrap();
        let mod_path = dir.path().join("mixed.cat");
        // META.exports list with non-string entry
        std::fs::write(&mod_path, "META.exports = [42, \"ok\"]\nok = 1").unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(Some(dir.path().to_path_buf()));
        pipeline.set_import_loader(loader);

        let err = pipeline.execute("import('mixed')").unwrap_err();
        assert!(
            err.to_string().contains("expected string") || err.to_string().contains("META.exports"),
            "expected error about non-string entry, got: {}",
            err
        );
    }

    #[test]
    fn test_io_write_returns_char_count() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unicode.txt");
        let path_str = path.to_str().unwrap();

        let mut pipeline = PurePipeline::new().unwrap();
        let loader = PureImportLoader::new(None);
        pipeline.set_import_loader(loader);

        // "café" is 4 chars but 5 bytes (é = 2 bytes)
        let code = format!(
            "import('io')\nf = io.open(\"{}\", \"w\")\nn = f.write(\"café\")\nf.close()\nn",
            path_str.replace('\\', "\\\\"),
        );
        let result = pipeline.execute(&code).unwrap();
        assert_eq!(
            result.as_int(),
            Some(4),
            "write() should return char count, not byte count"
        );
    }
}
