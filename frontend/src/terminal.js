const { Terminal } = require('xterm');

const term = new Terminal({
	cursorBlink: "block",
	// 25 rows 80 colls, the same as a default serial console.
	rows: 25,
	cols: 80,
	scrollback: 2000,
	logLevel: "off",
	minimumContractRatio: 7,
});

console.log(window.location);
console.log(window.location.host);
console.log(window.location.protocol);

// Ugly site construction
let site;
if (!window.location.host) {
	site = "localhost:9999";
} else {
	site = window.location.host;
}
// Set up websocket, override binary data type as we don't want blobs
const ws = new WebSocket("ws://" + site + "/ws");
ws.binaryType = "arraybuffer";

// Attach terminal
term.open(document.getElementById('terminal'));

ws.onmessage = msg => {
	term.write(new Uint8Array(msg.data));
};

// Use onData instead of onKey, this also fires when something is pasted
// into the console.
// onKey on the other hand fires when keys are pressed and seems to be
// used more to override individual key functionality.
term.onData(function(data, ev) {
	ws.send(data);
});
