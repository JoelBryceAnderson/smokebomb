# Zephyr application for the Sugarcube nRF54L15 board. The firmware is Rust:
# this builds it as a static library (packages/firmware/zephyr/rust) with
# cargo and links it into the app.
cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(smokebomb)

target_sources(app PRIVATE src/main.c src/shim.c)

set(SMOKEBOMB_ROOT ${CMAKE_CURRENT_SOURCE_DIR}/../../..)
set(SMOKEBOMB_RUST_TARGET thumbv8m.main-none-eabihf)
set(SMOKEBOMB_RUST_LIB
    ${SMOKEBOMB_ROOT}/target/${SMOKEBOMB_RUST_TARGET}/firmware/libsmokebomb_zephyr.a)
if(CONFIG_SMOKEBOMB_BENCH)
  set(SMOKEBOMB_RUST_FEATURES --features bench)
endif()

# Cargo decides what needs rebuilding, so run it on every build.
add_custom_target(smokebomb_rust
  COMMAND cargo build -p smokebomb-zephyr
          --target ${SMOKEBOMB_RUST_TARGET} --profile firmware
          ${SMOKEBOMB_RUST_FEATURES}
  WORKING_DIRECTORY ${SMOKEBOMB_ROOT}
  BYPRODUCTS ${SMOKEBOMB_RUST_LIB}
  USES_TERMINAL
)
add_library(smokebomb_rust_lib STATIC IMPORTED GLOBAL)
set_target_properties(smokebomb_rust_lib PROPERTIES IMPORTED_LOCATION ${SMOKEBOMB_RUST_LIB})
add_dependencies(app smokebomb_rust)
target_link_libraries(app PRIVATE smokebomb_rust_lib)
