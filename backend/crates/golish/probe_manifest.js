#!/usr/bin/env node

const https = require('https');
const http = require('http');

const targetUrl = 'http://8.138.179.62:8080';
const manifestPaths = [
  '/.vite/manifest.json',
  '/manifest.json',
  '/asset-manifest.json',
  '/stats.json',
  '/_next/static/chunks/_buildManifest.js',
  '/_next/static/chunks/_ssgManifest.js',
  '/_nuxt/builds/latest.json',
  '/_nuxt/manifest.json',
  '/ngsw.json',
];

function fetchManifest(url) {
  return new Promise((resolve) => {
    const client = url.startsWith('https') ? https : http;
    client.get(url, { rejectUnauthorized: false }, (res) => {
      let data = '';
      res.on('data', (chunk) => (data += chunk));
      res.on('end', () => resolve({ url, status: res.statusCode, data }));
    }).on('error', () => resolve({ url, status: 500, data: '' }));
  });
}

(async () => {
  const results = await Promise.all(manifestPaths.map(p => fetchManifest(`${targetUrl}${p}`)));
  const found = results.filter(r => r.status === 200);
  if (found.length > 0) {
    console.log('MANIFEST_FOUND:', JSON.stringify(found, null, 2));
    process.exit(0);
  } else {
    console.log('NO_MANIFEST');
    process.exit(1);
  }
})();
