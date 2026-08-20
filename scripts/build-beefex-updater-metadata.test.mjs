import { strict as assert } from 'node:assert'
import {
  buildUpdaterDocument,
  normalizeAlphaVersion,
  parseSha256sums,
} from './build-beefex-updater-metadata.mjs'

assert.equal(normalizeAlphaVersion('0.1.0-alpha.5'), '0.1.0-alpha.5')
assert.equal(normalizeAlphaVersion('v0.1.0-alpha.7'), '0.1.0-alpha.7')
assert.throws(() => normalizeAlphaVersion('0.1.0'), /0\.1\.0-alpha\.N/)
assert.throws(() => normalizeAlphaVersion('1.0.0-alpha.1'), /0\.1\.0-alpha\.N/)
assert.throws(() => normalizeAlphaVersion('0.1.0-beta.1'), /0\.1\.0-alpha\.N/)

const checksums = parseSha256sums(`
9887c1dddc735d39bd064473316ab4cb7cd6fe7ad4ccb3170025ba14c488b1c9  beefex-desktop-mac-arm64.dmg
38ba09c229f7beea1cac865ed1c22e54e48b45e6a3d4c2f43ee460d4ad2c1cee  *beefex-desktop-win-x64.exe
`)
assert.equal(
  checksums['beefex-desktop-mac-arm64.dmg'],
  '9887c1dddc735d39bd064473316ab4cb7cd6fe7ad4ccb3170025ba14c488b1c9',
)
assert.throws(() => parseSha256sums('not-a-hash  beefex-desktop-mac-arm64.dmg'), /malformed SHA line/)
assert.throws(() => parseSha256sums('9887c1dddc735d39bd064473316ab4cb7cd6fe7ad4ccb3170025ba14c488b1c9'), /malformed SHA line/)

const document = buildUpdaterDocument({
  version: '0.1.0-alpha.7',
  checksums,
})
assert.equal(document.version, '0.1.0-alpha.7')
assert.equal(document.tag, 'v0.1.0-alpha.7')
assert.equal(document.source_commit, '')

console.log('build-beefex-updater-metadata.selftest.ok')
