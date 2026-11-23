#!/bin/sh
# Start as root so an existing /data volume can be handed to the thrum user,
# then drop privileges before the server reads the database.
set -eu
# Everything the server creates is for the thrum user alone, and so is
# what an older image left in the volume.
umask 077
mkdir -p /data
chown -R thrum:thrum /data
chmod -R go-rwx /data
exec setpriv --reuid=thrum --regid=thrum --init-groups --inh-caps=-all -- thrum-server "$@"
