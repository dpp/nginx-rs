#!/bin/env bash

# Ensure that the NGINX_DIR variable is set
# then build the project, copy to the Nginx directory
# and run Nginx

if [ -z "$NGINX_DIR" ]
then
  echo "You must set the NGINX_DIR variable to the location of the Nginx source code."
  echo "For example 'export NGINX_DIR=~/projects/nginx'"
  exit 1
fi

echo "NGINX_DIR set to '$NGINX_DIR'"

if [ ! -d "$NGINX_DIR/r/modules" ] ; then
  echo "The '$NGINX_DIR/r/modules' directory does not exist, can't build/copy. Please run the 'setup.sh' script."
  exit 1
fi

(cargo build --release && cp target/release/lib*.so "$NGINX_DIR/r/modules") || exit 1

if [ -f "nginx.conf" ] ; then
  cp nginx.conf "$NGINX_DIR/r/conf/"
fi

echo
echo
echo "Nginx is running. Point your browser to your dev instance and check it out. Control-C to stop"

(cd "$NGINX_DIR" ; r/sbin/nginx)
