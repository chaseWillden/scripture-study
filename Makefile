# Scripture Study — common tasks. Run `make help` for a list.

CARGO   ?= cargo
APP     := scripture-study-desktop
BIN_DIR ?= $(HOME)/.local/bin

.DEFAULT_GOAL := help

.PHONY: help build release run test lint fmt snapshots clean cli

help: ## Show available targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  \033[36m%-10s\033[0m %s\n", $$1, $$2}'

build: ## Build all crates (debug)
	$(CARGO) build --workspace

release: ## Build an optimized desktop binary at target/release/scripture-study
	$(CARGO) build --release -p $(APP)

run: ## Run the desktop app (set SCRIPTURE_STUDY_DIR to choose where notes are stored)
	$(CARGO) run -p $(APP)

test: ## Run all tests (core unit tests + headless UI tests)
	$(CARGO) test --workspace

lint: ## Check formatting and run clippy
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt: ## Format all code
	$(CARGO) fmt --all

snapshots: ## Render UI screenshots from the tests into target/snapshots
	mkdir -p target/snapshots
	NOTES_SNAPSHOT_DIR=$(CURDIR)/target/snapshots $(CARGO) test -p $(APP)
	@echo "Snapshots written to target/snapshots"

clean: ## Remove build artifacts
	$(CARGO) clean

cli: release ## Symlink the release binary onto PATH as `scripture-study`
	mkdir -p $(BIN_DIR)
	ln -sfn $(CURDIR)/target/release/scripture-study $(BIN_DIR)/scripture-study
	@echo "Linked $(BIN_DIR)/scripture-study -> $(CURDIR)/target/release/scripture-study"
