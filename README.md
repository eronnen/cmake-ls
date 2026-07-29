# cmake-ls

`cmake-ls` prints the buildable targets in an existing CMake build tree.
Target names are sorted, deduplicated across configurations, and written one
per line, making the output suitable for interactive use and shell pipelines.
If a downstream command closes the output pipe early, `cmake-ls` terminates
without reporting a broken-pipe error.

## Requirements

- CMake 3.14 or newer available as `cmake` on `PATH`
- An existing configured build tree containing `CMakeCache.txt`

## Usage

```console
$ cmake-ls
app
core
generate
```

The optional positional argument selects a build directory:

```console
$ cmake-ls out/debug
```

When it is omitted, `cmake-ls` uses the first configured build tree found at
`./build`, `./build/debug`, or `./build/release`, in that order.

Use `cmake-ls --help` for the complete command-line interface.

## How it works

The command creates a client-owned CMake File API codemodel query at:

```text
<build>/.cmake/api/v1/query/client-cmake-ls/codemodel-v2
```

It also queries the File API's `cmakeFiles-v1` object, which lists the files
CMake used while configuring and generating. If the newest reply contains both
objects and none of those input files is newer than the reply, `cmake-ls`
reuses it without starting CMake. Projects with `CONFIGURE_DEPENDS` globs are
regenerated conservatively.

Otherwise, the command runs `cmake <build>` to regenerate the existing build
tree using its cached source directory, generator, toolchain, and cache options.
Routine CMake output is suppressed. If regeneration fails, the captured CMake
output is reported on stderr and the command exits unsuccessfully.

After regeneration, `cmake-ls` follows the query's reference from the newest
File API reply index and reads the codemodel. It lists executables, libraries,
and custom or utility targets represented in the codemodel's buildable target
list. Abstract imported targets and non-buildable interface libraries are not
included.

Use `cmake-ls --refresh` to regenerate unconditionally. This is useful for
project configuration logic that depends on the current environment or other
external state that is not represented by CMake's input-file list. As with
invoking CMake directly, regeneration can have side effects.

Pressing Ctrl+C stops the active CMake process group and exits with status 130.
On Unix, `cmake-ls` first forwards an interrupt and forcefully terminates the
group if it does not stop within two seconds or Ctrl+C is pressed again. On
platforms without graceful process-group interrupts, cancellation is immediate.

## Development

The crate denies compiler warnings and the Clippy `all`, `pedantic`, `nursery`,
and `cargo` lint groups. Run the complete local quality gate with:

```console
$ cargo fmt --check
$ cargo check --all-targets
$ cargo clippy --all-targets --all-features
$ cargo test --all-targets
$ RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```
