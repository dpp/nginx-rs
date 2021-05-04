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

echo "Nginx is configured and built. Copy your '.so' files to '$NGINX_DIR/r/modules'"
echo 
echo "Update '$NGINX_DIR/r/conf/nginx.conf' with a 'load_module' directive and other stuff to run your module"
echo
echo "To run Nginx, '(cd $NGINX_DIR ; r/sbin/nginx)'"
echo