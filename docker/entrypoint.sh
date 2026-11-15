#!/bin/sh
# Start as root so an existing /data volume can be handed to the thrum user,
# then drop privileges before the server reads the database.
set -eu
mkdir -p /data
chown -R thrum:thrum /data
exec setpriv --reuid=thrum --regid=thrum --init-groups --inh-caps=-all -- thrum-server "$@"
