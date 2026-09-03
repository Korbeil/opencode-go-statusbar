name := 'opencode-go-statusbar'
appid := 'dev.korbeil.opencode-go-statusbar'

rootdir := ''
prefix := '/usr'

base-dir := absolute_path(clean(rootdir / prefix))
user-dir := home_directory()
cargo-target-dir := env('CARGO_TARGET_DIR', 'target')
bin-src := cargo-target-dir / 'release' / name
desktop-src := 'resources' / appid + '.desktop'
icon-src := 'resources' / 'icon.svg'
icon-symbolic-src := 'resources' / 'icon-symbolic.svg'

# Default recipe which runs `just build-release`
default: build-release

# Runs `cargo clean`
clean:
    cargo clean

# Compiles with debug profile
build-debug *args:
    cargo build {{args}}

# Compiles with release profile
build-release *args: (build-debug '--release' args)

# Runs a clippy check
check *args:
    cargo clippy {{args}} -- -W clippy::pedantic

# Run the application for testing purposes
run *args:
    env RUST_BACKTRACE=full cargo run --release {{args}}

# Runs tests
test:
    cargo test

# Installs files system-wide (requires sudo for /usr)
install:
    install -Dm0755 {{bin-src}} {{base-dir}}/bin/{{name}}
    install -Dm0644 {{desktop-src}} {{base-dir}}/share/applications/{{appid}}.desktop
    install -Dm0644 {{icon-src}} {{base-dir}}/share/icons/hicolor/scalable/apps/{{appid}}.svg
    install -Dm0644 {{icon-symbolic-src}} {{base-dir}}/share/icons/hicolor/scalable/status/{{appid}}-symbolic.svg

# Uninstalls system-wide files
uninstall:
    rm -f {{base-dir}}/bin/{{name}} \
        {{base-dir}}/share/applications/{{appid}}.desktop \
        {{base-dir}}/share/icons/hicolor/scalable/apps/{{appid}}.svg \
        {{base-dir}}/share/icons/hicolor/scalable/status/{{appid}}-symbolic.svg

# Installs files into the user's home directory
install-user:
    install -Dm0755 {{bin-src}} {{user-dir}}/.local/bin/{{name}}
    install -Dm0644 {{desktop-src}} {{user-dir}}/.local/share/applications/{{appid}}.desktop
    install -Dm0644 {{icon-src}} {{user-dir}}/.local/share/icons/hicolor/scalable/apps/{{appid}}.svg
    install -Dm0644 {{icon-symbolic-src}} {{user-dir}}/.local/share/icons/hicolor/scalable/status/{{appid}}-symbolic.svg

# Uninstalls user files
uninstall-user:
    rm -f {{user-dir}}/.local/bin/{{name}} \
        {{user-dir}}/.local/share/applications/{{appid}}.desktop \
        {{user-dir}}/.local/share/icons/hicolor/scalable/apps/{{appid}}.svg \
        {{user-dir}}/.local/share/icons/hicolor/scalable/status/{{appid}}-symbolic.svg
