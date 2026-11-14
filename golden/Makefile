# Regenerates the golden fixtures (see README.md).
#
#   make            Tier B (Blender) then Tier A (oracle) for every case
#   make tier-b     demo scans + decimated meshes only
#   make tier-a     oracle stages on the committed scans (needs python3 + numpy)
#   make case-demo_lower_mild   one case, Tier A only
#   make check      round-trip and size checks of what is committed
#   make clean-stages  delete the per-stage outputs (keeps scan/decimated/meta)
#
# Variables: PLUGIN (read-only plugin repository), BLENDER, PYTHON, JOBS (parallel cases).

PLUGIN  ?= $(HOME)/Documents/repos/blender-ortho-aligner
BLENDER ?= blender
PYTHON  ?= python3
JOBS    ?= 6
HERE    := $(dir $(abspath $(lastword $(MAKEFILE_LIST))))
FIX     := $(HERE)fixtures
CASES   := demo_upper_mild demo_upper_moderate demo_upper_severe demo_lower_mild demo_lower_moderate demo_lower_severe

export PYTHONDONTWRITEBYTECODE := 1
export OA_PLUGIN := $(PLUGIN)

.PHONY: all tier-a tier-b check clean-stages $(addprefix case-,$(CASES))
all: tier-b tier-a

tier-b:
	$(BLENDER) -b --factory-startup -P $(HERE)blender_dump.py -- --out $(FIX) --plugin $(PLUGIN)

tier-a: $(addprefix case-,$(CASES))

$(addprefix case-,$(CASES)): case-%:
	$(PYTHON) -B $(HERE)oracle.py all --in $(FIX)/$* --plugin $(PLUGIN)

# the cases are independent: `make -j6 tier-a`
check:
	$(PYTHON) -B $(HERE)check.py $(FIX)

clean-stages:
	for c in $(CASES); do for s in archform worldframe autoseed segmentation teeth diagnosis setup staging schedule classify; do rm -rf $(FIX)/$$c/$$s; done; done
