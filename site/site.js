const repo = 'https://api.github.com/repos/getshepherd/shepherd'

// The hero button offers this machine's build; the row's own button is the fallback link until the API answers.
const ua = navigator.userAgent
const os = /Mac|iPhone|iPad/.test(ua) ? 'macos' : /Windows/.test(ua) ? 'windows' : /Linux|X11/.test(ua) ? 'linux' : null
const hero = document.getElementById('hero-download')
if (os) {
  hero.dataset.downloadOs = os
  hero.textContent = 'Download for ' + { macos: 'macOS', windows: 'Windows', linux: 'Linux' }[os]
  hero.href = document.querySelector(`[data-os="${os}"]`).href
}

// Point each download at the newest release that has its file; without the API the links open the releases page.
fetch(repo + '/releases?per_page=10')
  .then(r => r.ok ? r.json() : [])
  .then(releases => {
    releases = releases.filter(r => !r.draft && !r.prerelease)
    if (!releases.length) return
    const v = document.querySelector('.version')
    v.textContent = 'Latest release: ' + releases[0].tag_name + '.'
    v.hidden = false
    for (const a of document.querySelectorAll('[data-asset]')) {
      const re = new RegExp(a.dataset.asset)
      for (const r of releases) {
        const f = r.assets.find(f => re.test(f.name))
        if (f) { a.href = f.browser_download_url; a.title = f.name + ', ' + Math.round(f.size / 1e6) + ' MB'; break }
      }
    }
    if (os) hero.href = document.querySelector(`[data-os="${os}"]`).href
  })
  .catch(() => {})

fetch(repo)
  .then(r => r.ok ? r.json() : null)
  .then(j => {
    if (!j || !j.stargazers_count) return
    const s = document.querySelector('.gh-stars')
    s.textContent = '★ ' + (j.stargazers_count >= 1000 ? (j.stargazers_count / 1000).toFixed(1) + 'k' : j.stargazers_count)
    s.hidden = false
  })
  .catch(() => {})

// A headline is split into its two lines' words, so the CSS can light the first line and raise the second word by
// word; the heading keeps its text for a screen reader as a label.
for (const h of document.querySelectorAll('.head h1, .head h2')) {
  const lines = [h.firstElementChild.textContent, h.lastChild.textContent].map(t => t.trim().split(/\s+/))
  h.setAttribute('aria-label', lines.flat().join(' '))
  h.textContent = ''
  lines.forEach((words, n) => {
    const line = h.appendChild(document.createElement('span'))
    line.className = n ? 'ans' : 'dim'
    line.ariaHidden = 'true'
    words.forEach((word, i) => {
      const w = line.appendChild(document.createElement('span'))
      w.className = 'w'
      w.style.setProperty('--i', i)
      w.textContent = word + ' '
    })
  })
}

// Ordered dither, the page's one texture: a canvas is cut into cells and a cell is lit where `value` (0..1 at the
// cell, given its place as fractions of the canvas and as cell indices) passes the 4×4 Bayer threshold.
const bayer = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5]
function dither(canvas, cell, color, value) {
  const dpr = devicePixelRatio || 1
  const w = canvas.clientWidth, h = canvas.clientHeight
  if (!w || !h) return
  canvas.width = w * dpr; canvas.height = h * dpr
  const c = canvas.getContext('2d')
  c.scale(dpr, dpr)
  c.fillStyle = color
  for (let j = 0; j * cell < h; j++)
    for (let i = 0; i * cell < w; i++)
      if (value(i * cell / w, j * cell / h, i, j) > (bayer[(j & 3) * 4 + (i & 3)] + 0.5) / 16)
        c.fillRect(i * cell, j * cell, cell - 1, cell - 1)
}

// A card's field: slow waves that thin out towards the middle, where the card's content lies. Each card gets its own
// phase, so no two look alike.
function drawFields() {
  document.querySelectorAll('.dither').forEach((canvas, n) => {
    dither(canvas, 3, '#465064', (x, y) => {
      const edge = Math.max(Math.abs(x - 0.5), Math.abs(y - 0.5)) * 2
      const wave = 0.5 + 0.5 * Math.sin(x * 6 + n * 1.7 + Math.sin(y * 5 + n) * 2.2)
      return edge * edge * (0.25 + 0.6 * wave)
    })
  })
}

// The footer's name: the word is drawn small, one pixel per cell, and each covered cell is dithered, dense on the
// left and thinning to the right. The canvas takes the word's height less its descender, so the page ends mid-letter.
const mark = document.querySelector('.wordmark')
function drawMark() {
  const cell = mark.clientWidth < 700 ? 3 : 5
  const cols = Math.floor(mark.clientWidth / cell)
  const small = document.createElement('canvas')
  const sc = small.getContext('2d', { willReadFrequently: true })
  sc.font = '600 100px Geist, system-ui, sans-serif'
  sc.letterSpacing = '-4px'
  const size = 100 * cols / sc.measureText('Shepherd').width
  const rows = Math.ceil(size * 0.86)
  small.width = cols; small.height = rows
  sc.font = `600 ${size}px Geist, system-ui, sans-serif`
  sc.letterSpacing = -0.04 * size + 'px'
  sc.fillText('Shepherd', 0, size * 0.76)
  const alpha = sc.getImageData(0, 0, cols, rows).data
  mark.style.height = rows * cell + 'px'
  dither(mark, cell, '#88c0d0', (x, y, i, j) => i < cols && j < rows && alpha[(j * cols + i) * 4 + 3] > 127 ? 0.97 - 0.6 * x : 0)
}

function draw() { drawFields(); drawMark() }
document.fonts ? document.fonts.ready.then(draw) : draw()
addEventListener('resize', draw)
