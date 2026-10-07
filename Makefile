CARGO ?= cargo
INSTALL_DIR ?= $(HOME)/.local/bin

.PHONY: cli-build cli-test cli-install

cli-build:
	$(CARGO) build --release --manifest-path rust/Cargo.toml --bin doc-pointers

cli-test:
	$(CARGO) test --manifest-path rust/Cargo.toml

cli-install: cli-build
	install -d "$(INSTALL_DIR)"
	install -m 755 rust/target/release/doc-pointers "$(INSTALL_DIR)/doc-pointers"
