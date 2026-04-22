# Canary Probe — Go reference capability build
#
# Prerequisites:
#   - TinyGo 0.34.0+: https://tinygo.org/getting-started/install/
#   - wasm-tools: cargo install wasm-tools
#
# Usage:
#   make component   # Build WASM component (requires TinyGo)
#   make test        # Native test build (standard Go)
#   make clean       # Remove build artifacts

NAME = canary-probe

.PHONY: component test clean

component:
	tinygo build -target=wasip2 -o $(NAME).wasm .
	wasm-tools component new $(NAME).wasm -o $(NAME).component.wasm
	@rm -f $(NAME).wasm

test:
	go run main.go

clean:
	rm -f $(NAME).wasm $(NAME).component.wasm $(NAME).manifest.cbor
