#!/bin/sh
# Run a Blender-side script headless, with factory settings and a private user dir in the work folder.
#
#   SCRIPT=plugin_run.py ./runblender.sh op     all cases through the real import operator
#   SCRIPT=plugin_run.py ./runblender.sh direct all cases through the internal helpers
#   SCRIPT=names_run.py ./runblender.sh          role-from-name table (group C)
#
# Environment: BLENDER (default: blender on PATH), IMPORT_DIFF_WORK (see paths.py).
set -eu
SP=$(cd "$(dirname "$0")" && pwd)
WORK=${IMPORT_DIFF_WORK:-${TMPDIR:-/tmp}/dentalcraft-import-diff}
mkdir -p "$WORK"
export PYTHONDONTWRITEBYTECODE=1
export BLENDER_USER_CONFIG=$WORK/blender_home/config BLENDER_USER_SCRIPTS=$WORK/blender_home/scripts \
  BLENDER_USER_DATAFILES=$WORK/blender_home/datafiles BLENDER_USER_EXTENSIONS=$WORK/blender_home/extensions
exec "${BLENDER:-blender}" --background --factory-startup -P "$SP/${SCRIPT:-plugin_run.py}" -- "$@"
