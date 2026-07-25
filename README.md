# cmake-ls

`cmake-ls` prints the buildable targets in an existing CMake build tree.
Target names are sorted, deduplicated across configurations, and written one
per line, making the output suitable for interactive use and shell pipelines.

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

The optional positional argument selects a build directory and defaults to
`./build`:

```console
$ cmake-ls out/debug
```

Use `cmake-ls --help` for the complete command-line interface.

## How it works

The command creates a client-owned CMake File API codemodel query at:

```text
<build>/.cmake/api/v1/query/client-cmake-ls/codemodel-v2
```

It then runs `cmake <build>` to regenerate the existing build tree using its
cached source directory, generator, toolchain, and cache options. Routine CMake
output is suppressed. If regeneration fails, the captured CMake output is
reported on stderr and the command exits unsuccessfully.

After regeneration, `cmake-ls` follows the query's reference from the newest
File API reply index and reads the codemodel. It lists executables, libraries,
and custom or utility targets represented in the codemodel's buildable target
list. Abstract imported targets and non-buildable interface libraries are not
included.

Running `cmake-ls` re-executes the project's configure and generate steps. As
with invoking CMake directly, project configuration logic can have side effects
or depend on the current environment.

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
