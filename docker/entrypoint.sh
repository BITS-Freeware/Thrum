#!/bin/sh
# Start as root only long enough to hand a new /data volume to the thrum
# user, then drop privileges before the server reads the database.
set -eu
# Everything the server creates is for the thrum user alone.
umask 077
mkdir -p /data
# Re-own the volume only when it is not the thrum user's yet, so a host
# directory mounted here by mistake is not re-owned on every start.
if [ "$(stat -c %u /data)" != "10001" ]; then
  chown -R thrum:thrum /data
fi
drop="setpriv --reuid=thrum --regid=thrum --init-groups --inh-caps=-all --no-new-privs"
# Tighten what an older image left in the volume, as the thrum user.
$drop -- chmod -R go-rwx /data 2>/dev/null ||
  echo "warning: some files in /data are not the thrum user's; left as they are" >&2
exec $drop -- thrum-server "$@"
