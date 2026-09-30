# Zephyr application shell for the Sugarcube nRF54L15 board.
# TODO: link the Rust firmware as a static library (smokebomb-firmware,
# crate-type = "staticlib", target thumbv8m.main-none-eabihf).
cmake_minimum_required(VERSION 3.20.0)
find_package(Zephyr REQUIRED HINTS $ENV{ZEPHYR_BASE})
project(smokebomb)

target_sources(app PRIVATE src/main.c)
