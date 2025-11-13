use std::env;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=ffi/splash_bridge.cc");
    println!("cargo:rerun-if-changed=ffi/splash_bridge.h");

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

    let dst = configure_and_build_poppler(&poppler_src);

    compile_bridge(&manifest_dir, &poppler_src, &dst);

    let lib_dir = dst.join("lib");
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=poppler");
    // println!("cargo:rustc-link-lib=static=poppler-splash");

    // Poppler requires these system libraries even in a minimal build.
    println!("cargo:rustc-link-lib=freetype");
    println!("cargo:rustc-link-lib=fontconfig");
    println!("cargo:rustc-link-lib=jpeg");
    println!("cargo:rustc-link-lib=openjp2");
    println!("cargo:rustc-link-lib=png");
    println!("cargo:rustc-link-lib=tiff");
    println!("cargo:rustc-link-lib=iconv");
    println!("cargo:rustc-link-lib=z");
    emit_system_library_hints();

    // macOS uses libc++ for the standard library implementation.
    println!("cargo:rustc-link-lib=c++");
}

fn configure_and_build_poppler(poppler_src: &Path) -> PathBuf {
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
        .define("ENABLE_LCMS", "OFF")
        .define("ENABLE_NSS3", "OFF")
        .define("ENABLE_GPGME", "OFF")
        .define("ENABLE_PGP_SIGNATURES", "OFF")
        .define("ENABLE_BOOST", "ON")
        .define("ENABLE_GTK_DOC", "OFF")
        .define("ENABLE_LIBOPENJPEG", "openjpeg2")
        .define("ENABLE_DCTDECODER", "libjpeg")
        .define("ENABLE_ZLIB_UNCOMPRESS", "OFF")
        .define("FONT_CONFIGURATION", "fontconfig")
        .define("BUILD_GTK_TESTS", "OFF")
        .define("BUILD_QT5_TESTS", "OFF")
        .define("BUILD_QT6_TESTS", "OFF")
        .define("BUILD_CPP_TESTS", "OFF")
        .define("BUILD_MANUAL_TESTS", "OFF")
        .define("RUN_GPERF_IF_PRESENT", "OFF");

    cfg.build()
}

fn compile_bridge(manifest_dir: &Path, poppler_src: &Path, dst: &Path) {
    let include_dir = dst.join("include");
    let include_dir_poppler = include_dir.join("poppler");

    let mut build = cc::Build::new();
    build
        .cpp(true)
        .file(manifest_dir.join("ffi/splash_bridge.cc"))
        .include(manifest_dir.join("ffi"))
        .include(&include_dir)
        .include(&include_dir_poppler)
        .include(poppler_src.join("poppler"))
        .include(poppler_src.join("goo"))
        .include(poppler_src.join("splash"))
        .include(poppler_src.join("fofi"))
        .flag_if_supported("-std=c++20")
        .flag_if_supported("-Wno-unused-parameter")
        .flag_if_supported("-Wno-unused-variable");

    build.compile("tiny_poppler_splash_bridge");
}

fn emit_system_library_hints() {
    let libraries = [
        ("FREETYPE_DIR", "freetype"),
        ("FONTCONFIG_DIR", "fontconfig"),
        ("JPEG_DIR", "jpeg-turbo"),
        ("PNG_DIR", "libpng"),
        ("TIFF_DIR", "libtiff"),
        ("OPENJPEG_DIR", "openjpeg"),
    ];

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
