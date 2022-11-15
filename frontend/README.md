# Cloud Console Frontend

This directory contains the frontend of Cloud Console. It is just a simple web page, which hosts a terminal, and connects to the server over a websocket connection.

## Building

After the initial clone, run `npm install` to download the required dependencies.

Source files are hosted in the `src` folder. In order to build the packaged files,
run `npm run build`. This will package the files and place them in the `dist` folder.
Note that the script removes the `.map` files, if these are needed you will need
to edit the script in [package.json](./package.json).

## Supported npm versions

Tested with npm version

- 12.20.2
- 16.18.1

Other versions _should_ work, but are currently untested
