// libfiles — talk to file FORCE from a Quick Action script.
//
//   const files = require('libfiles');
//   const answers = files.ask('Resize', [files.choice('size', 'Width', ['1080', '720'])]);
//   files.made(output); files.notify('Done', '3 pictures resized');
//
// Messages go to Files as JSON lines on stdout; answers come back on stdin.
'use strict';
const fs = require('fs');
const path = require('path');

class Cancelled extends Error {}

function send(message) {
  fs.writeSync(1, '@files ' + JSON.stringify(message) + '\n');
}

function readLine() {
  const bytes = [];
  const one = Buffer.alloc(1);
  while (fs.readSync(0, one, 0, 1, null) === 1 && one[0] !== 10) bytes.push(one[0]);
  return Buffer.from(bytes).toString('utf8');
}

module.exports = {
  Cancelled,
  selection: () => process.argv.slice(2),
  answer: (key, fallback) => process.env['FILES_ANSWER_' + key.toUpperCase()] ?? fallback,
  choice: (key, label, options) => ({ kind: 'choice', key, label, options }),
  text: (key, label, def = '') => ({ kind: 'text', key, label, default: def }),
  number: (key, label, def = 0) => ({ kind: 'number', key, label, default: String(def) }),
  ask(title, fields) {
    send({ op: 'ask', title, fields });
    const reply = JSON.parse(readLine() || '{"cancelled":true}');
    if (reply.cancelled) throw new Cancelled('cancelled');
    return reply.answers || {};
  },
  notify: (title, body = null, level = 'success') => send({ op: 'notify', title, body, level }),
  made: (file) => send({ op: 'made', path: path.resolve(file) }),
  reveal: (file) => send({ op: 'reveal', path: path.resolve(file) }),
  progress: (text) => send({ op: 'progress', text }),
};
