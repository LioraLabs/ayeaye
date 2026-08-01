#!/usr/bin/env node
const fs = require('fs');
const vm = require('vm');

const html = fs.readFileSync(process.argv[2], 'utf8');
const start = html.indexOf('function markdownInline');
const end = html.indexOf('\nfunction card(', start);
if(start < 0 || end < 0) throw new Error('markdown renderer not found');

global.KEYS_RE = null;
global.esc = s => s.replace(/&/g,'&amp;').replace(/</g,'&lt;')
                   .replace(/>/g,'&gt;').replace(/"/g,'&quot;');
global.linkify = s => esc(s);
vm.runInThisContext(html.slice(start, end));

const rendered = markdownHTML(`# Heading

**clear** and \`literal\`

| Name | State |
| --- | --- |
| alpha | **ready** |

- first
- second`);

for(const expected of [
  '<h3>Heading</h3>', '<strong>clear</strong>', '<code>literal</code>',
  '<div class="md-table"><table>', '<th>Name</th>',
  '<td><strong>ready</strong></td>', '<ul><li>first</li><li>second</li></ul>'
]){
  if(!rendered.includes(expected))
    throw new Error(`missing ${expected}\n${rendered}`);
}

const hostile = markdownHTML('<img src=x onerror=alert(1)>');
if(hostile.includes('<img')) throw new Error('markdown emitted raw HTML');
