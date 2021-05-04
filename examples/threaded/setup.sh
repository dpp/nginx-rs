#!/bin/env bash

# Ensure that the NGINX_DIR variable is set
# and that nginx is set up, and built locally

if [ -z "$NGINX_DIR" ]
then
  echo "You must set the NGINX_DIR variable to the location of the Nginx source code."
  echo "For example 'export NGINX_DIR=~/projects/nginx'"
  exit 1
fi

echo "NGINX_DIR set to '$NGINX_DIR'"

pushd "$NGINX_DIR"

./auto/configure  --with-compat --prefix=r && make && make install && mkdir -p r/modules && popd

echo "Nginx is configured and built. Use the 'smoke_test.sh' script to build your module and run Nginx"
echo 