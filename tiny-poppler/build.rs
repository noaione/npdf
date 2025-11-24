use std::env;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sanitizer {
    Address,
    Leak,
    Thread,
    Undefined,
}

impl Sanitizer {
    fn detect(target: &str) -> Result<Option<Self>, String> {
        let mut enabled = Vec::new();

        if env::var_os("CARGO_FEATURE_ASAN").is_some() {
            enabled.push(Sanitizer::Address);
        }
        if env::var_os("CARGO_FEATURE_LSAN").is_some() {
            enabled.push(Sanitizer::Leak);
        }
        if env::var_os("CARGO_FEATURE_TSAN").is_some() {
            enabled.push(Sanitizer::Thread);
        }
        if env::var_os("CARGO_FEATURE_UBSAN").is_some() {
            enabled.push(Sanitizer::Undefined);
        }

        if enabled.len() > 1 {
            return Err("more than one sanitizer feature enabled; choose exactly one".to_string());
        }

        let sanitizer = enabled.into_iter().next();

        if let Some(choice) = sanitizer {
            if target.contains("windows-msvc") {
                return Err(format!(
                    "{} sanitizer is not supported on MSVC targets",
                    choice.display_name()
                ));
            }

            if target.contains("apple") && matches!(choice, Sanitizer::Leak) {
                return Err(format!(
                    "{} sanitizer is not available on Apple targets",
                    choice.display_name()
                ));
            }
        }

        Ok(sanitizer)
    }

    fn display_name(self) -> &'static str {
        match self {
            Sanitizer::Address => "AddressSanitizer",
            Sanitizer::Leak => "LeakSanitizer",
            Sanitizer::Thread => "ThreadSanitizer",
            Sanitizer::Undefined => "UndefinedBehaviorSanitizer",
        }
    }

    fn compile_flags(self) -> &'static [&'static str] {
        match self {
            Sanitizer::Address => &["-fsanitize=address", "-fno-omit-frame-pointer"],
            Sanitizer::Leak => &["-fsanitize=leak", "-fno-omit-frame-pointer"],
            Sanitizer::Thread => &["-fsanitize=thread"],
            Sanitizer::Undefined => &["-fsanitize=undefined"],
        }
    }

    fn link_args(self) -> &'static [&'static str] {
        match self {
            Sanitizer::Address => &["-fsanitize=address"],
            Sanitizer::Leak => &["-fsanitize=leak"],
            Sanitizer::Thread => &["-fsanitize=thread"],
            Sanitizer::Undefined => &["-fsanitize=undefined"],
        }
    }

    fn cfg_value(self) -> &'static str {
        match self {
            Sanitizer::Address => "address",
            Sanitizer::Leak => "leak",
            Sanitizer::Thread => "thread",
            Sanitizer::Undefined => "undefined",
        }
    }

    fn unix_runtime_lib(self) -> Option<&'static str> {
        match self {
            Sanitizer::Address => Some("asan"),
            Sanitizer::Leak => Some("lsan"),
            Sanitizer::Thread => Some("tsan"),
            Sanitizer::Undefined => Some("ubsan"),
        }
    }

    fn apple_runtime_dylib(self) -> &'static str {
        match self {
            Sanitizer::Address => "libclang_rt.asan_osx_dynamic.dylib",
            Sanitizer::Leak => "libclang_rt.lsan_osx_dynamic.dylib",
            Sanitizer::Thread => "libclang_rt.tsan_osx_dynamic.dylib",
            Sanitizer::Undefined => "libclang_rt.ubsan_osx_dynamic.dylib",
        }
    }

    fn apple_runtime_lib_name(self) -> &'static str {
        match self {
            Sanitizer::Address => "clang_rt.asan_osx_dynamic",
            Sanitizer::Leak => "clang_rt.lsan_osx_dynamic",
            Sanitizer::Thread => "clang_rt.tsan_osx_dynamic",
            Sanitizer::Undefined => "clang_rt.ubsan_osx_dynamic",
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=ffi/splash_bridge.cc");
    println!("cargo:rerun-if-changed=ffi/splash_bridge.h");
    println!("cargo:rerun-if-changed=ffi/exporter_bridge.cc");
    println!("cargo:rerun-if-changed=ffi/exporter_bridge.h");
    println!("cargo:rerun-if-changed=ffi/image_exporter.cc");
    println!("cargo:rerun-if-changed=ffi/image_exporter.h");
    println!("cargo:rerun-if-changed=ffi/splash_renderer_internal.h");

    // Rerun if ../third_party/poppler/poppler folder changes
    let current_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let parent_dir = Path::new(&current_dir)
        .parent()
        .expect("crate should reside inside workspace root");
    let poppler_path = parent_dir.join("third_party/poppler/poppler");
    // iterate all cc and h files
    for entry in walkdir::WalkDir::new(&poppler_path) {
        let entry = entry.expect("failed to read file in poppler source tree");
        if entry.file_type().is_file() {
            println!("cargo:rerun-if-changed={}", entry.path().display());
        }
    }

    let target = env::var("TARGET").expect("TARGET not set");

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let workspace_root = manifest_dir
        .parent()
        .expect("crate should reside inside workspace root");
    let poppler_src = workspace_root.join("third_party/poppler");

    if !poppler_src.exists() {
        panic!(
            "expected Poppler sources under third_party/poppler; run git submodule update --init"
        );
    }

    println!(
        "cargo:rerun-if-changed={}",
        poppler_src.join("CMakeLists.txt").display()
    );

    let sanitizer = Sanitizer::detect(&target)
        .unwrap_or_else(|err| panic!("sanitizer configuration error: {err}"));

    if let Some(selected) = sanitizer {
        println!(
            "cargo:warning=Enabling {} instrumentation",
            selected.display_name()
        );
        println!("cargo:rustc-cfg=tiny_poppler_has_sanitizer");
        println!(
            "cargo:rustc-cfg=tiny_poppler_sanitizer=\"{}\"",
            selected.cfg_value()
        );
    }

    let dst = configure_and_build_poppler(&poppler_src, &target, sanitizer);

    compile_bridge(&manifest_dir, &poppler_src, &dst, sanitizer);

    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=poppler");
    // println!("cargo:rustc-link-lib=static=poppler-splash");

    emit_linker_flags(&target, sanitizer);
}

fn configure_and_build_poppler(
    poppler_src: &Path,
    target: &str,
    sanitizer: Option<Sanitizer>,
) -> PathBuf {
    let is_windows = target.contains("windows");
    let mut cfg = cmake::Config::new(poppler_src);

    // Check build profile
    let build_profile = match std::env::var("PROFILE") {
        Ok(str) => {
            if str == "release" {
                "Release"
            } else {
                "Debug"
            }
        }
        Err(_) => panic!("Unknown build profile"),
    };

    println!("Building with profile: {build_profile}");

    cfg.profile(build_profile)
        .static_crt(false)
        .define("CMAKE_POSITION_INDEPENDENT_CODE", "ON")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("ENABLE_UNSTABLE_API_ABI_HEADERS", "ON")
        .define("ENABLE_UTILS", "OFF")
        .define("ENABLE_CPP", "ON")
        .define("ENABLE_GLIB", "OFF")
        .define("ENABLE_GOBJECT_INTROSPECTION", "OFF")
        .define("ENABLE_QT5", "OFF")
        .define("ENABLE_QT6", "OFF")
        .define("ENABLE_LIBCURL", "OFF")
        .define("ENABLE_LIBTIFF", "ON")
        .define("ENABLE_LCMS", "ON")
        .define("ENABLE_GPGME", "OFF")
        .define("ENABLE_PGP_SIGNATURES", "OFF")
        .define("ENABLE_BOOST", "ON")
        .define("ENABLE_GTK_DOC", "OFF")
        .define("ENABLE_LIBOPENJPEG", "openjpeg2")
        .define("ENABLE_DCTDECODER", "libjpeg")
        .define("ENABLE_ZLIB_UNCOMPRESS", "OFF")
        .define("BUILD_GTK_TESTS", "OFF")
        .define("BUILD_QT5_TESTS", "OFF")
        .define("BUILD_QT6_TESTS", "OFF")
        .define("BUILD_CPP_TESTS", "OFF")
        .define("BUILD_MANUAL_TESTS", "OFF")
        .define("RUN_GPERF_IF_PRESENT", "OFF");

    if let Ok(ci_build) = std::env::var("CI_RUNNER") {
        println!("Running on CI: {ci_build}");
        cfg.define("CMAKE_VERBOSE_MAKEFILE", "ON");
    }

    if !is_windows && let Some(s) = sanitizer {
        apply_sanitizer_to_cmake(&mut cfg, s);
    }

    if target.contains("windows") {
        cfg.define("FONT_CONFIGURATION", "win32")
            .define("ENABLE_NSS3", "OFF");

        if let Ok(vcpkg_root) = std::env::var("VCPKG_ROOT") {
            let toolchain = Path::new(&vcpkg_root).join("scripts/buildsystems/vcpkg.cmake");
            if toolchain.exists() {
                println!("Using vcpkg toolchain: {}", toolchain.display());
                cfg.define("CMAKE_TOOLCHAIN_FILE", toolchain);
            }
        }

        if let Ok(triplet) = std::env::var("VCPKG_DEFAULT_TRIPLET") {
            cfg.define("VCPKG_TARGET_TRIPLET", triplet);
        }

        cfg.cxxflag("-DWIN32_LEAN_AND_MEAN").cxxflag("-DNOMINMAX");
    } else {
        cfg.define("FONT_CONFIGURATION", "fontconfig")
            .define("ENABLE_NSS3", "ON");
    }

    cfg.build()
}

fn compile_bridge(
    manifest_dir: &Path,
    poppler_src: &Path,
    dst: &Path,
    sanitizer: Option<Sanitizer>,
) {
    let is_windows = env::var("TARGET")
        .map(|t| t.contains("windows"))
        .unwrap_or(false);
    let include_dir = dst.join("include");
    let include_dir_poppler = include_dir.join("poppler");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .static_crt(false)
        .file(manifest_dir.join("ffi/splash_bridge.cc"))
        .file(manifest_dir.join("ffi/exporter_bridge.cc"))
        .file(manifest_dir.join("ffi/image_exporter.cc"))
        .include(manifest_dir.join("ffi"))
        .include(&include_dir)
        .include(&include_dir_poppler)
        .include(poppler_src.join("poppler"))
        .include(poppler_src.join("goo"))
        .include(poppler_src.join("splash"))
        .include(poppler_src.join("fofi"))
        .define("WIN32_LEAN_AND_MEAN", None)
        .define("NOMINMAX", None)
        .flag_if_supported("-std=c++23")
        .flag_if_supported("/std:c++latest") // For MSVC (should be C++23 preview, requires VS 2022 17.14)
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-variable");

    if !is_windows && let Some(s) = sanitizer {
        apply_sanitizer_to_cc(&mut build, s);
    }

    build.compile("tiny_poppler_splash_bridge");
}

fn emit_system_library_hints(target: &str) {
    let mut libraries = vec![
        ("FREETYPE_DIR", "freetype"),
        ("JPEG_DIR", "jpeg-turbo"),
        ("PNG_DIR", "libpng"),
        ("TIFF_DIR", "libtiff"),
        ("OPENJPEG_DIR", "openjpeg"),
        ("LCMS2_DIR", "lcms2"),
    ];

    if !target.contains("windows") {
        libraries.push(("FONTCONFIG_DIR", "fontconfig"));
    }

    if !target.contains("windows") {
        libraries.push(("NSS_DIR", "nss"));
        libraries.push(("NSPR_DIR", "nspr"));
    }

    for (env_var, formula) in libraries {
        if let Some(dir) = env::var_os(env_var).map(PathBuf::from) {
            let lib_dir = dir.join("lib");
            if lib_dir.exists() {
                println!("cargo:rustc-link-search=native={}", lib_dir.display());
                continue;
            }
        }

        emit_homebrew_search_hint(formula);
    }
}

fn emit_linker_flags(target: &str, sanitizer: Option<Sanitizer>) {
    let is_windows = target.contains("windows");
    let is_apple = target.contains("apple");

    if !is_windows && let Some(s) = sanitizer {
        emit_sanitizer_link_args(target, s);
    }

    let loonix_search = vec![
        "/usr/lib/x86_64-linux-gnu",
        "/usr/local/lib/x86_64-linux-gnu",
        "/usr/lib",
        "/usr/local/lib",
    ];
    let some_commons = vec!["turbojpeg", "openjp2", "tiff"];

    if is_apple {
        emit_homebrew_search_hint("bzip2");
        emit_homebrew_search_hint("brotli");
        emit_homebrew_search_hint("xz");
        emit_homebrew_search_hint("expat");
    }

    println!("cargo:rustc-link-lib=static=bz2");
    println!("cargo:rustc-link-lib=static=brotlidec");
    println!("cargo:rustc-link-lib=static=brotlienc");
    println!("cargo:rustc-link-lib=static=brotlicommon");
    println!("cargo:rustc-link-lib=static=lzma");

    if is_windows {
        // Do static linking
        println!("cargo:rustc-link-lib=static=freetype");
        for lib in some_commons {
            println!("cargo:rustc-link-lib=static={}", lib);
        }
        println!("cargo:rustc-link-lib=static=lcms2");
        println!("cargo:rustc-link-lib=static=libpng16");
        println!("cargo:rustc-link-lib=static=zlib");
        println!("cargo:rustc-link-lib=dylib=advapi32");
        println!("cargo:rustc-link-lib=dylib=shell32");
        println!("cargo:rustc-link-lib=dylib=shfolder");
    } else {
        println!("cargo:rustc-link-lib=static=expat");
        println!("cargo:rustc-link-lib=static=freetype");
        for lib in some_commons {
            println!("cargo:rustc-link-lib=static={}", lib);
        }
        if is_apple {
            println!("cargo:rustc-link-lib=static=lcms2");
        } else {
            println!("cargo:rustc-link-lib=lcms2");
        }
        println!("cargo:rustc-link-lib=static=png");
        println!("cargo:rustc-link-lib=z");

        println!("cargo:rustc-link-lib=static=fontconfig");
        println!("cargo:rustc-link-lib=nss3");
        println!("cargo:rustc-link-lib=nssutil3");
        println!("cargo:rustc-link-lib=smime3");
        println!("cargo:rustc-link-lib=ssl3");
        println!("cargo:rustc-link-lib=static=plds4");
        println!("cargo:rustc-link-lib=static=plc4");
        println!("cargo:rustc-link-lib=static=nspr4");
    }

    if !is_apple && !is_windows {
        // Try searching in common Linux library path
        for path in loonix_search {
            println!("cargo:rustc-link-search=native={}", path);
        }
    }

    if is_apple {
        println!("cargo:rustc-link-lib=iconv");
        println!("cargo:rustc-link-lib=c++");
    } else if is_windows {
        // MSVC links the C++ runtime automatically; no additional flags required.
    } else {
        println!("cargo:rustc-link-lib=stdc++");
    }

    if is_windows {
        if let Ok(root) = env::var("VCPKG_ROOT") {
            let triplet =
                env::var("VCPKG_DEFAULT_TRIPLET").unwrap_or("x64-windows-static".to_string());
            let lib_path = PathBuf::from(root)
                .join("installed")
                .join(triplet)
                .join("lib");
            println!("cargo:rustc-link-search=native={}", lib_path.display());
        }
    } else {
        emit_system_library_hints(target);
    }
}

fn apply_sanitizer_to_cmake(cfg: &mut cmake::Config, sanitizer: Sanitizer) {
    let compile_flags = sanitizer.compile_flags();
    for flag in compile_flags {
        cfg.cflag(flag);
        cfg.cxxflag(flag);
    }

    let link_flags = sanitizer.link_args();
    if !link_flags.is_empty() {
        let joined = link_flags.join(" ");
        cfg.define("CMAKE_EXE_LINKER_FLAGS", &joined);
        cfg.define("CMAKE_SHARED_LINKER_FLAGS", &joined);
        cfg.define("CMAKE_MODULE_LINKER_FLAGS", &joined);
    }
}

fn apply_sanitizer_to_cc(build: &mut cc::Build, sanitizer: Sanitizer) {
    for flag in sanitizer.compile_flags() {
        build.flag(flag);
    }
}

fn emit_sanitizer_link_args(target: &str, sanitizer: Sanitizer) {
    let is_apple = target.contains("apple");
    let is_windows = target.contains("windows");

    if is_windows {
        println!(
            "cargo:warning={} sanitizer is not supported on Windows targets",
            sanitizer.display_name()
        );
        return;
    }

    if is_apple {
        emit_macos_sanitizer_runtime(target, sanitizer);
        return;
    }

    for arg in sanitizer.link_args() {
        println!("cargo:rustc-link-arg={}", arg);
    }

    if let Some(lib) = sanitizer.unix_runtime_lib() {
        println!("cargo:rustc-link-lib={}", lib);
    }
}

fn emit_macos_sanitizer_runtime(target: &str, sanitizer: Sanitizer) {
    let mut builder = cc::Build::new();
    if !target.is_empty() {
        builder.target(target);
    }

    let compiler = builder.get_compiler();
    let dylib = sanitizer.apple_runtime_dylib();

    let resolved = {
        let mut cmd = compiler.to_command();
        cmd.arg(format!("-print-file-name={dylib}"));
        cmd.output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                if path.is_empty() || path == dylib {
                    None
                } else {
                    Some(PathBuf::from(path))
                }
            })
    };

    let path = match resolved {
        Some(path) => path,
        None => {
            println!(
                "cargo:warning=Unable to locate {dylib}; make sure Xcode Command Line Tools are installed"
            );
            return;
        }
    };

    if let Some(parent) = path.parent() {
        let dir = parent.display().to_string();
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-arg=-Wl,-rpath,{dir}");
    }
    println!(
        "cargo:rustc-link-lib={}",
        sanitizer.apple_runtime_lib_name()
    );
}

fn emit_homebrew_search_hint(formula: &str) {
    let candidates = [
        env::var_os("HOMEBREW_PREFIX").map(PathBuf::from),
        env::var_os("BREW_PREFIX").map(PathBuf::from),
        Some(PathBuf::from("/opt/homebrew")),
        Some(PathBuf::from("/usr/local")),
    ];

    for prefix in candidates.into_iter().flatten() {
        let candidate = prefix.join("opt").join(formula).join("lib");
        if candidate.exists() {
            println!("cargo:rustc-link-search=native={}", candidate.display());
            break;
        }
    }
}
