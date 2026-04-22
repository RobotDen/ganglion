# Log normalization capability — Python reference build
#
# Prerequisites:
#   pip install componentize-py
#
# Usage:
#   make component   # Build WASM component
#   make sign        # Sign with gang identity
#   make clean       # Remove build artifacts

NAME = log-normalize
WIT_DIR = wit

.PHONY: component sign test clean

component: $(NAME).component.wasm

$(NAME).component.wasm: app.py $(WIT_DIR)/ganglion.wit
	componentize-py -d $(WIT_DIR) -w ganglion-capability \
		componentize app -o $@

sign: $(NAME).component.wasm
	gang sign $< --name $(NAME) --version 0.1.0

test:
	python3 -c "from app import normalize_batch; \
		r = normalize_batch(['[INFO] [123] [/n]: hi']); \
		assert r['parsed_lines'] == 1; \
		print('PASS')"

clean:
	rm -f $(NAME).component.wasm $(NAME).manifest.cbor
