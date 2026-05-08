#!/usr/bin/make -f

CONTAINER_RUNTIME := $(shell which docker 2>/dev/null || which podman 2>/dev/null)

.PHONY: all
all: clean fmt lint test schema optimize

.PHONY: all-arm
all-arm: clean fmt lint test schema optimize-arm

.PHONY: clean
clean:
	@cargo clean

# TODO: Revisit adding doc back to all/all-arm when rustdoc "invalid fragment length" / search-index.js
# issue is fixed or worked around (e.g. doc only specific packages).
.PHONY: doc
doc:
	@cargo doc

.PHONY: fmt
fmt:
	@cargo fmt --all -- --check

.PHONY: lint
lint:
	@cargo clippy

.PHONY: build
build:
	@cargo build

.PHONY: test
test:
	@cargo test

.PHONY: schema
schema:
	@cargo run -p democratized-prime-pool-v2 --example schema
	@cargo run -p democratized-prime-price-oracle --example schema
	@cargo run -p repo-token-cw20 --example schema

.PHONY: coverage
coverage:
	@cargo tarpaulin --ignore-tests --out Html

.PHONY: optimize
optimize:
	$(CONTAINER_RUNTIME) run --rm -v $(CURDIR):/code:Z \
        --mount type=volume,source=democratized_prime_cache,target=/target \
        --mount type=volume,source=democratized_prime_registry_cache,target=/usr/local/cargo/registry \
        cosmwasm/optimizer:0.17.0

.PHONY: optimize-test
optimize-test:
	$(CONTAINER_RUNTIME) run --rm -v $(CURDIR):/code:Z \
	    -e TESTING=1 \
        --mount type=volume,source=democratized_prime_cache,target=/target \
        --mount type=volume,source=democratized_prime_registry_cache,target=/usr/local/cargo/registry \
        cosmwasm/optimizer:0.17.0

.PHONY: optimize-arm
optimize-arm:
	$(CONTAINER_RUNTIME) run --rm -v $(CURDIR):/code:Z \
        --mount type=volume,source=democratized_prime_cache,target=/target \
        --mount type=volume,source=democratized_prime_registry_cache,target=/usr/local/cargo/registry \
        cosmwasm/rust-optimizer-arm64:0.17.0


.PHONY: install
install: optimize
	@cp artifacts/democratized_prime_pool_v2.wasm $(PIO_HOME)
	@cp artifacts/repo_token_cw20.wasm $(PIO_HOME)
	@cp artifacts/democratized_prime_price_oracle.wasm $(PIO_HOME)

.PHONY: install-arm
install-arm: optimize-arm
	@cp artifacts/democratized_prime_pool_v2.wasm $(PIO_HOME)
	@cp artifacts/repo_token_cw20.wasm $(PIO_HOME)
	@cp artifacts/democratized_prime_price_oracle.wasm $(PIO_HOME)
