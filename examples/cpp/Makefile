# Topic Echo — C++ reference capability build
#
# Prerequisites:
#   - wasi-sdk (v26+): export WASI_SDK_PATH=/opt/wasi-sdk
#   - wit-bindgen CLI: cargo install wit-bindgen-cli
#   - wasm-tools: cargo install wasm-tools
#
# Usage:
#   make component   # Full WASM component build (requires wasi-sdk)
#   make test        # Native test build (host compiler)
#   make clean       # Remove build artifacts

NAME = topic-echo
WASI_SDK ?= $(WASI_SDK_PATH)
CC = $(WASI_SDK)/bin/clang++
WIT_DIR = wit

.PHONY: component bindings test clean

# Step 1: Generate C bindings from WIT
bindings: $(WIT_DIR)/ganglion.wit
	wit-bindgen c $(WIT_DIR) --world ganglion-capability --out-dir gen

# Step 2+3: Compile and create component
component: bindings
	$(CC) --target=wasm32-wasip1 -mexec-model=reactor -fno-exceptions \
		-I gen -o $(NAME).core.wasm \
		gen/ganglion_capability.c gen/ganglion_capability_component_type.o \
		src/main.cpp
	wasm-tools component embed $(WIT_DIR) $(NAME).core.wasm \
		-o $(NAME).embedded.wasm
	wasm-tools component new $(NAME).embedded.wasm \
		-o $(NAME).component.wasm
	@rm -f $(NAME).core.wasm $(NAME).embedded.wasm

# Native test (no wasi-sdk needed)
test: src/main.cpp
	$(CXX) -DSTANDALONE_TEST -o $(NAME)-test src/main.cpp
	./$(NAME)-test
	@rm -f $(NAME)-test

clean:
	rm -rf gen/
	rm -f $(NAME).core.wasm $(NAME).embedded.wasm $(NAME).component.wasm
	rm -f $(NAME).manifest.cbor $(NAME)-test
