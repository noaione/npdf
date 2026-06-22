cmake_minimum_required(VERSION 3.28)
project(tiny_poppler_core LANGUAGES CXX C)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

if(NOT CMAKE_BUILD_TYPE AND NOT CMAKE_CONFIGURATION_TYPES)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()

option(BUILD_POPPLER "Build Poppler from the bundled third_party/poppler sources" ON)
option(POPPLER_ROOT "Path to a pre-built Poppler installation (used when BUILD_POPPLER=OFF)")

# Resolve Poppler source directory relative to this project.
set(POPPLER_SRC "${CMAKE_CURRENT_SOURCE_DIR}/../third_party/poppler")
if(NOT EXISTS "${POPPLER_SRC}/CMakeLists.txt")
    message(FATAL_ERROR "Poppler sources not found at ${POPPLER_SRC}. "
                        "Run: git submodule update --init --recursive")
endif()

if(BUILD_POPPLER)
    set(POPPLER_BUILD "${CMAKE_CURRENT_BINARY_DIR}/poppler-build")
    set(POPPLER_INSTALL "${CMAKE_CURRENT_BINARY_DIR}/poppler-install")

    # CMake configuration mirroring tiny-poppler/build.rs as closely as possible.
    # Use Unix Makefiles for the nested Poppler build to avoid relying on the
    # ephemeral ninja binary provided by the isolated build environment.
    set(POPPLER_CMAKE_ARGS
        -G "Unix Makefiles"
        -S ${POPPLER_SRC}
        -B ${POPPLER_BUILD}
        -DCMAKE_BUILD_TYPE=${CMAKE_BUILD_TYPE}
        -DCMAKE_POSITION_INDEPENDENT_CODE=ON
        -DBUILD_SHARED_LIBS=OFF
        -DENABLE_UNSTABLE_API_ABI_HEADERS=ON
        -DENABLE_UTILS=OFF
        -DENABLE_CPP=ON
        -DENABLE_GLIB=OFF
        -DENABLE_GOBJECT_INTROSPECTION=OFF
        -DENABLE_QT5=OFF
        -DENABLE_QT6=OFF
        -DENABLE_LIBCURL=OFF
        -DENABLE_LIBTIFF=ON
        -DENABLE_LCMS=ON
        -DENABLE_GPGME=OFF
        -DENABLE_PGP_SIGNATURES=OFF
        -DENABLE_BOOST=ON
        -DENABLE_GTK_DOC=OFF
        -DENABLE_LIBOPENJPEG=openjpeg2
        -DENABLE_LIBJPEG=ON
        -DENABLE_ZLIB_UNCOMPRESS=OFF
        -DBUILD_GTK_TESTS=OFF
        -DBUILD_QT5_TESTS=OFF
        -DBUILD_QT6_TESTS=OFF
        -DBUILD_CPP_TESTS=OFF
        -DBUILD_MANUAL_TESTS=OFF
        -DRUN_GPERF_IF_PRESENT=OFF
        -DCMAKE_INSTALL_PREFIX=${POPPLER_INSTALL}
    )

    if(APPLE)
        list(APPEND POPPLER_CMAKE_ARGS -DFONT_CONFIGURATION=fontconfig -DENABLE_NSS3=ON)
    elseif(WIN32)
        list(APPEND POPPLER_CMAKE_ARGS -DFONT_CONFIGURATION=win32 -DENABLE_NSS3=OFF)
        # Match the static CRT preference from build.rs if requested.
        if(MSVC)
            if(CMAKE_MSVC_RUNTIME_LIBRARY STREQUAL "MultiThreaded" OR
               CMAKE_MSVC_RUNTIME_LIBRARY STREQUAL "MultiThreadedDebug")
                list(APPEND POPPLER_CMAKE_ARGS -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded)
            else()
                list(APPEND POPPLER_CMAKE_ARGS -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreadedDLL)
            endif()
        endif()
        if(DEFINED ENV{VCPKG_ROOT})
            set(VCPKG_ROOT "$ENV{VCPKG_ROOT}")
            set(VCPKG_TOOLCHAIN "${VCPKG_ROOT}/scripts/buildsystems/vcpkg.cmake")
            if(EXISTS "${VCPKG_TOOLCHAIN}")
                list(APPEND POPPLER_CMAKE_ARGS -DCMAKE_TOOLCHAIN_FILE=${VCPKG_TOOLCHAIN})
            endif()
            if(DEFINED ENV{VCPKG_DEFAULT_TRIPLET})
                list(APPEND POPPLER_CMAKE_ARGS -DVCPKG_TARGET_TRIPLET=$ENV{VCPKG_DEFAULT_TRIPLET})
            endif()
        endif()
    else()
        list(APPEND POPPLER_CMAKE_ARGS -DFONT_CONFIGURATION=fontconfig -DENABLE_NSS3=ON)
    endif()

    message(STATUS "Configuring Poppler static build...")
    execute_process(
        COMMAND ${CMAKE_COMMAND} ${POPPLER_CMAKE_ARGS}
        RESULT_VARIABLE poppler_config_result
    )
    if(NOT poppler_config_result EQUAL 0)
        message(FATAL_ERROR "Poppler CMake configuration failed (code ${poppler_config_result})")
    endif()

    message(STATUS "Building Poppler (this may take a few minutes)...")
    execute_process(
        COMMAND ${CMAKE_COMMAND} --build ${POPPLER_BUILD} --config ${CMAKE_BUILD_TYPE} --parallel
        RESULT_VARIABLE poppler_build_result
    )
    if(NOT poppler_build_result EQUAL 0)
        message(FATAL_ERROR "Poppler build failed (code ${poppler_build_result})")
    endif()

    message(STATUS "Installing Poppler into ${POPPLER_INSTALL}...")
    execute_process(
        COMMAND ${CMAKE_COMMAND} --install ${POPPLER_BUILD} --config ${CMAKE_BUILD_TYPE}
        RESULT_VARIABLE poppler_install_result
    )
    if(NOT poppler_install_result EQUAL 0)
        message(FATAL_ERROR "Poppler install failed (code ${poppler_install_result})")
    endif()

    set(POPPLER_ROOT "${POPPLER_INSTALL}")
endif()

# Locate the static Poppler library and headers.
find_library(POPPLER_LIBRARY
    NAMES poppler libpoppler
    PATHS "${POPPLER_ROOT}/lib" "${POPPLER_ROOT}/lib64"
    NO_DEFAULT_PATH
    REQUIRED
)
find_path(POPPLER_INCLUDE_DIR
    NAMES poppler-config.h
    PATHS "${POPPLER_ROOT}/include" "${POPPLER_ROOT}/include/poppler"
    NO_DEFAULT_PATH
    REQUIRED
)

message(STATUS "Poppler library: ${POPPLER_LIBRARY}")
message(STATUS "Poppler includes: ${POPPLER_INCLUDE_DIR}")

# Create an imported Poppler target so we can attach transitive dependencies.
add_library(poppler_static STATIC IMPORTED)
set_target_properties(poppler_static PROPERTIES IMPORTED_LOCATION "${POPPLER_LIBRARY}")
# POPPLER_INCLUDE_DIR points at <prefix>/include/poppler (Object.h, GfxState.h, ...).
# The bridge files also include <poppler/poppler-config.h>, so add the parent dir.
get_filename_component(POPPLER_BASE_INCLUDE_DIR "${POPPLER_INCLUDE_DIR}" DIRECTORY)
target_include_directories(poppler_static INTERFACE
    ${POPPLER_INCLUDE_DIR}
    ${POPPLER_BASE_INCLUDE_DIR}
)

# Dependency discovery. On macOS we lean on pkg-config/Homebrew; on Linux on
# pkg-config/system paths; on Windows on vcpkg (which is already part of Poppler's
# own CMake configuration).
find_package(PkgConfig REQUIRED)
find_package(Freetype REQUIRED)
find_package(JPEG REQUIRED)
find_package(PNG REQUIRED)
find_package(TIFF REQUIRED)
find_package(OpenJPEG REQUIRED)

pkg_check_modules(LCMS2 REQUIRED IMPORTED_TARGET lcms2)

set(TINY_POPPLER_LINK_LIBS poppler_static Freetype::Freetype JPEG::JPEG PNG::PNG TIFF::TIFF PkgConfig::LCMS2)

if(OpenJPEG_TARGETS OR TARGET openjp2)
    list(APPEND TINY_POPPLER_LINK_LIBS openjp2)
elseif(TARGET OpenJPEG::OpenJPEG)
    list(APPEND TINY_POPPLER_LINK_LIBS OpenJPEG::OpenJPEG)
else()
    # Fallback: assume pkg-config knows about libopenjp2.
    pkg_check_modules(OPENJP2 REQUIRED libopenjp2)
    list(APPEND TINY_POPPLER_LINK_LIBS ${OPENJP2_LIBRARIES})
    set(TINY_POPPLER_EXTRA_INCLUDES ${OPENJP2_INCLUDE_DIRS})
endif()

set(TINY_POPPLER_PLATFORM_INCLUDES "")
if(APPLE)
    pkg_check_modules(FONTCONFIG REQUIRED IMPORTED_TARGET fontconfig)
    pkg_check_modules(NSS3 REQUIRED IMPORTED_TARGET nss>=3.98)
    pkg_check_modules(NSPR REQUIRED IMPORTED_TARGET nspr)
    pkg_check_modules(BROTLIDEC REQUIRED IMPORTED_TARGET libbrotlidec)
    pkg_check_modules(BROTLIENC REQUIRED IMPORTED_TARGET libbrotlienc)
    pkg_check_modules(BROTLICOMMON REQUIRED IMPORTED_TARGET libbrotlicommon)
    pkg_check_modules(BZIP2 REQUIRED IMPORTED_TARGET bzip2)
    pkg_check_modules(LIBLZMA REQUIRED IMPORTED_TARGET liblzma)
    pkg_check_modules(EXPAT REQUIRED IMPORTED_TARGET expat)

    find_library(ICONV_LIBRARY iconv)

    list(APPEND TINY_POPPLER_LINK_LIBS
        PkgConfig::FONTCONFIG
        PkgConfig::NSS3
        PkgConfig::NSPR
        PkgConfig::BZIP2
        PkgConfig::BROTLIDEC
        PkgConfig::BROTLIENC
        PkgConfig::BROTLICOMMON
        PkgConfig::LIBLZMA
        PkgConfig::EXPAT
        ZLIB::ZLIB
        ${ICONV_LIBRARY}
        c++
    )
elseif(WIN32)
    list(APPEND TINY_POPPLER_LINK_LIBS
        freetype
        openjp2
        turbojpeg
        lcms2
        libpng16
        zlib
        advapi32
        shell32
        shfolder
    )
else() # Linux and other Unix
    pkg_check_modules(FONTCONFIG REQUIRED IMPORTED_TARGET fontconfig)
    pkg_check_modules(NSS3 REQUIRED IMPORTED_TARGET nss>=3.98)
    pkg_check_modules(NSPR REQUIRED IMPORTED_TARGET nspr)
    pkg_check_modules(BROTLIDEC REQUIRED IMPORTED_TARGET libbrotlidec)
    pkg_check_modules(BROTLIENC REQUIRED IMPORTED_TARGET libbrotlienc)
    pkg_check_modules(BROTLICOMMON REQUIRED IMPORTED_TARGET libbrotlicommon)
    pkg_check_modules(BZIP2 REQUIRED IMPORTED_TARGET bzip2)
    pkg_check_modules(LIBLZMA REQUIRED IMPORTED_TARGET liblzma)
    pkg_check_modules(EXPAT REQUIRED IMPORTED_TARGET expat)

    list(APPEND TINY_POPPLER_LINK_LIBS
        PkgConfig::FONTCONFIG
        PkgConfig::NSS3
        PkgConfig::NSPR
        PkgConfig::BZIP2
        PkgConfig::BROTLIDEC
        PkgConfig::BROTLIENC
        PkgConfig::BROTLICOMMON
        PkgConfig::LIBLZMA
        PkgConfig::EXPAT
        ZLIB::ZLIB
        stdc++
    )
endif()

# Some internal Poppler headers (e.g. JBIG2Stream.h) are not installed even when
# unstable headers are enabled, so also make the source tree available as a
# fallback include path.
set(POPPLER_SRC_INCLUDE_DIR "${POPPLER_SRC}/poppler")

# Build the pure C++ Poppler bridge as a static library so its headers are not
# exposed to Python's Object.h include path.
add_library(tiny_poppler_ffi STATIC
    ffi/splash_bridge.cc
    ffi/exporter_bridge.cc
    ffi/image_exporter.cc
    ffi/splash_renderer_internal.cc
)
target_include_directories(tiny_poppler_ffi PRIVATE
    ${POPPLER_INCLUDE_DIR}
    ${POPPLER_BASE_INCLUDE_DIR}
    ${POPPLER_SRC_INCLUDE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/ffi
    ${LCMS2_INCLUDE_DIRS}
    ${TINY_POPPLER_EXTRA_INCLUDES}
    ${TINY_POPPLER_PLATFORM_INCLUDES}
)
target_link_libraries(tiny_poppler_ffi PRIVATE ${TINY_POPPLER_LINK_LIBS})

# Python + nanobind
find_package(Python 3.11 REQUIRED COMPONENTS Interpreter Development.Module)
find_package(nanobind CONFIG REQUIRED)

nanobind_add_module(
    _core
    bindings/bindings.cc
)
# Put Poppler includes *before* Python includes so that bridge headers such as
# splash_bridge.h resolve "Object.h" to Poppler's Object.h.
target_include_directories(_core BEFORE PRIVATE
    ${POPPLER_INCLUDE_DIR}
    ${POPPLER_BASE_INCLUDE_DIR}
    ${POPPLER_SRC_INCLUDE_DIR}
    ${CMAKE_CURRENT_SOURCE_DIR}/ffi
    ${LCMS2_INCLUDE_DIRS}
    ${TINY_POPPLER_EXTRA_INCLUDES}
    ${TINY_POPPLER_PLATFORM_INCLUDES}
)
target_link_libraries(_core PRIVATE ${TINY_POPPLER_LINK_LIBS} tiny_poppler_ffi)

install(TARGETS _core LIBRARY DESTINATION .)

# Generate type stubs for the nanobind extension at build time.
nanobind_add_stub(
    tiny_poppler_stub
    MODULE tiny_poppler._core
    OUTPUT ${CMAKE_CURRENT_SOURCE_DIR}/src/tiny_poppler/_core.pyi
    MARKER_FILE ${CMAKE_CURRENT_SOURCE_DIR}/src/tiny_poppler/py.typed
    PYTHON_PATH $<TARGET_FILE_DIR:_core>
    DEPENDS _core tiny_poppler_ffi
    VERBOSE
)
