"""Locations for the scan-import differential harness.

Defaults are relative to this folder. Environment variables override them:

  IMPORT_DIFF_WORK       scratch folder for fixtures, cases and outputs
                         (default: <system temp dir>/dentalcraft-import-diff)
  DENTALCRAFT_CLI        built dentalcraft-cli binary
                         (default: ../../../dentalcraft-app/target/debug/dentalcraft-cli)
  BLENDER_ORTHO_ALIGNER  Blender add-on repository, the read-only oracle
                         (default: ../../../../blender-ortho-aligner)
  BLENDER                Blender executable (default: blender on PATH)
"""
import os
import tempfile

SP = os.path.dirname(os.path.abspath(__file__))
GOLDEN = os.path.dirname(SP)
SUPERPROJECT = os.path.abspath(os.path.join(SP, '..', '..', '..'))

WORK = os.environ.get('IMPORT_DIFF_WORK') or os.path.join(tempfile.gettempdir(), 'dentalcraft-import-diff')
FX = os.path.join(WORK, 'fixtures')
OUT = os.path.join(WORK, 'out')
CASES = os.path.join(WORK, 'cases.json')

REF_DCM = os.path.join(GOLDEN, 'fixtures', 'demo_upper_mild', 'scan.dcm')
CLI = os.environ.get('DENTALCRAFT_CLI') or os.path.join(
    SUPERPROJECT, 'dentalcraft-app', 'target', 'debug', 'dentalcraft-cli')
PLUGIN = os.environ.get('BLENDER_ORTHO_ALIGNER') or os.path.abspath(
    os.path.join(SUPERPROJECT, '..', 'blender-ortho-aligner'))
