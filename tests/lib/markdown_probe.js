#!/usr/bin/env node
const fs = require('fs');
const vm = require('vm');

const html = fs.readFileSync(process.argv[2], 'utf8');
const start = html.indexOf('const FILE_EXT_RE');
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

function includes(value, part, label) {
  if(!value.includes(part)) throw new Error(`${label}: missing ${part}\n${value}`);
}

const refs = markdownHTML(
  '[the guide](docs/guide.md:7), `src/deep/module`, `Cookfile`, src/app.js:19, README.md; '+
  'but ordinary prose and changelog stay plain. **See docs/api.md.**');
includes(refs, 'class="file-ref"', 'file references become controls');
includes(refs, 'data-reference="docs/guide.md:7"', 'markdown destination is retained');
includes(refs, '>the guide</button>', 'markdown label composes with file link');
includes(refs, 'data-reference="src/deep/module"', 'backticked extensionless path');
includes(refs, 'data-reference="Cookfile"', 'backticks make an extensionless filename explicit');
includes(refs, '>src/app.js:19</button>', 'path and line remain literal');
includes(refs, '>README.md</button>', 'recognizable bare filename');
includes(refs, '<strong>See <button', 'strong markdown composes with file links');
if(/data-reference="(?:ordinary|prose|changelog)"/.test(refs))
  throw new Error('ordinary extensionless prose became a file reference');

const boundary = markdownInline('version 1.2.3, example.com, UTF-8, SHA-256, foo.xyzzy, HTML/CSS, read/write, and TCP/IP');
if(boundary.includes('file-ref')) throw new Error('false positive boundary became a file reference');

const hostileRef = markdownInline('src/&lt;bad&gt;.js and [x](a&quot;b.md)');
if(hostileRef.includes('data-reference="src/<') || hostileRef.includes('data-reference="a"b'))
  throw new Error('file reference attribute was not escaped');

const unique = filePreviewState({candidates:[{path:'docs/a.md',kind:'text'}],line:4});
if(unique.view !== 'preview' || unique.path !== 'docs/a.md' || unique.line !== 4)
  throw new Error('unique result does not immediately preview');
const ambiguous = filePreviewState({candidates:Array.from({length:25},(_,i)=>({path:`d/${i}.txt`}))});
if(ambiguous.view !== 'matches' || ambiguous.candidates.length !== 20)
  throw new Error('ambiguous results are not bounded to twenty matches');
if(filePreviewState({candidates:[]}).view !== 'empty')
  throw new Error('no match does not use quiet empty state');
if(filePreviewBack({view:'preview',candidates:[{path:'a'},{path:'b'}]}).view !== 'matches')
  throw new Error('back does not return to other matches');

for(const required of [
  'id="fileModal"', 'aria-modal="true"', 'id="fileClose"',
  'function closeFilePreview', 'back.focus()',
  "document.body.classList.remove('preview-open')",
  "fetch('/api/files/resolve'", "method:'POST'",
  "'/api/files/preview?'", "document.createElement('img')",
  '.file-modal', 'overflow:auto', '@media (max-width: 520px)',
  'white-space:pre', 'textContent = line.number'
]) includes(html, required, 'preview UI contract');
if(/innerHTML\s*=.*svg/i.test(html.slice(html.indexOf('function renderFilePreview'))))
  throw new Error('preview renderer may inject SVG markup');

const python = highlightCode('src/example.py', 'def greet(name): # hello\n  return f"Hi {name}"');
includes(python, '<span class="syn-key">def</span>', 'python keyword highlighting');
includes(python, '<span class="syn-comment"># hello</span>', 'python comment highlighting');
includes(python, '<span class="syn-string">f&quot;Hi {name}&quot;</span>', 'python string highlighting');
const json = highlightCode('config.json', '{"enabled": true, "count": 3}');
includes(json, '<span class="syn-string">&quot;enabled&quot;</span>', 'json strings highlighted');
includes(json, '<span class="syn-key">true</span>', 'json literals highlighted');
includes(json, '<span class="syn-number">3</span>', 'numbers highlighted');
const unsafeCode = highlightCode('x.js', '<img onerror="bad">');
if(unsafeCode.includes('<img')) throw new Error('syntax highlighter emitted raw HTML');
if(highlightCode('notes.unknown', 'plain < text') !== 'plain &lt; text')
  throw new Error('unknown syntax does not safely fall back to plain text');
