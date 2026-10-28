default: check

[private]
run-sub dir *recipe="":
    @if [ -n "${GITHUB_ACTIONS:-}" ]; then \
        echo "::group::{{dir}} {{recipe}}"; \
    elif [ -n "{{recipe}}" ]; then \
        printf "\n\033[1;36m━━━ [%s] just %s ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\033[0m\n" "{{dir}}" "{{recipe}}"; \
    else \
        printf "\n\033[1;36m━━━ [%s] ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\033[0m\n" "{{dir}}"; \
    fi
    @just -f {{dir}}/Justfile {{recipe}}
    @if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::endgroup::"; fi

check: check-nix-fmt check-typos test clippy
    cargo fmt --check
    @just run-sub examples/pc check
    @just run-sub examples/pi-pico check

build:
    cargo build
    @just run-sub examples/pc build
    @just run-sub examples/pi-pico build

check-nix-fmt:
    alejandra -q -c .

check-typos:
    typos

test:
    cargo test

clippy:
    cargo clippy --no-default-features --features no-std --features defmt -- -D warnings
    cargo clippy --no-default-features --features std --features log -- -D warnings
    cargo clippy --no-default-features --features no-std -- -D warnings
    cargo clippy --no-default-features --features std -- -D warnings

fmt:
    alejandra -q .
    cargo fmt
