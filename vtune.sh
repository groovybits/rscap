#!/bin/sh
#
# Install Intel vtune to use this
#
#
source /opt/intel/oneapi/vtune//latest/vtune-vars.sh
vtune-backend --web-port 8088 \
    --allow-remote-access \
    --enable-server-profiling \
    --log-to-console \
    --log-level info \
    --reset-passphrase \
    --suppress-automatic-help-tours \
        --data-directory ~/vtune
