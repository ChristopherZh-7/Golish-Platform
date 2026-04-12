#!/usr/bin/env node

const fs = require('fs');

// Read the saved HTML
const html = fs.readFileSync('target_page.html', 'utf8');

// Extract script tags
const scripts = [];
const scriptRegex = /<script[^>]+src="([^"]+)"[^>]*>/g;
let match;
while ((match = scriptRegex.exec(html)) !== null) {
  scripts.push(match[1]);
}

console.log('SCRIPTS:', JSON.stringify(scripts, null, 2));
