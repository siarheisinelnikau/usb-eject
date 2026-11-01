const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

module.exports = async ({ github, context, core }) => {
  const directory = '.release-stage';
  const release = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json'), 'utf8'));
  const subject = context.payload.head_commit?.message.split('\n')[0].replace(/\r$/, '');
  if (context.eventName !== 'push' || context.ref !== 'refs/heads/main' ||
      subject !== `Release localization v${release.version}` || release.commit !== context.sha ||
      !/^\d+\.\d+\.\d+$/.test(release.version) || release.tag !== `v${release.version}`) {
    throw new Error('The explicit release commit, tag and checked artifacts must match.');
  }
  const notes = fs.readFileSync(release.notes, 'utf8');
  if (notes.length < 100 || !notes.includes(release.version)) throw new Error('Missing reviewed notes for this version.');
  const names = new Set();
  for (const asset of release.assets) {
    if (path.basename(asset.name) !== asset.name || names.has(asset.name)) throw new Error('Unsafe or duplicate asset name.');
    names.add(asset.name);
    const data = fs.readFileSync(path.join(directory, asset.name));
    const digest = `sha256:${crypto.createHash('sha256').update(data).digest('hex')}`;
    if (!data.length || data.length !== asset.size || digest !== asset.digest) throw new Error(`Staged asset changed: ${asset.name}`);
  }
  const primaryAsset = release.kind === 'source' ?
    `${release.product}-v${release.version}-source.zip` : `${release.product}-v${release.version}.exe`;
  if (!names.has(primaryAsset) || !names.has('SHA256SUMS.txt')) {
    throw new Error('Missing canonical release asset or checksums.');
  }
  const repository = context.repo;
  async function assertTagTarget() {
    let target;
    try {
      target = (await github.rest.git.getRef({ ...repository, ref: `tags/${release.tag}` })).data.object;
    } catch (error) { if (error.status === 404) return; throw error; }
    for (let depth = 0; target.type === 'tag' && depth < 8; depth++) {
      target = (await github.rest.git.getTag({ ...repository, tag_sha: target.sha })).data.object;
    }
    if (target.type !== 'commit' || target.sha !== context.sha) throw new Error('An existing version tag belongs to another commit.');
  }
  await assertTagTarget();
  let draft;
  try {
    draft = (await github.rest.repos.getReleaseByTag({ ...repository, tag: release.tag })).data;
  } catch (error) { if (error.status !== 404) throw error; }
  if (draft && (!draft.draft || draft.target_commitish !== context.sha || draft.prerelease)) {
    throw new Error('A published release or a draft for another commit will not be replaced.');
  }
  if (draft) {
    for (const existing of draft.assets) {
      const expected = release.assets.find(asset => asset.name === existing.name);
      if (!expected || existing.size !== expected.size || existing.digest !== expected.digest) {
        throw new Error(`An existing draft asset differs: ${existing.name}`);
      }
    }
  }
  if (!draft) {
    draft = (await github.rest.repos.createRelease({
      ...repository, tag_name: release.tag, target_commitish: context.sha,
      name: `${release.product} ${release.tag}`, body: notes, draft: true, prerelease: false,
    })).data;
  }
  for (const asset of release.assets) {
    if (draft.assets.some(existing => existing.name === asset.name)) continue;
    const data = fs.readFileSync(path.join(directory, asset.name));
    const uploaded = (await github.rest.repos.uploadReleaseAsset({
      ...repository, release_id: draft.id, name: asset.name, data,
      headers: { 'content-type': 'application/octet-stream', 'content-length': data.length },
    })).data;
    if (uploaded.size !== asset.size || uploaded.digest !== asset.digest) {
      throw new Error(`GitHub asset verification failed; release remains a draft: ${asset.name}`);
    }
  }
  const ready = (await github.rest.repos.getRelease({ ...repository, release_id: draft.id })).data;
  if (ready.assets.length !== release.assets.length) throw new Error('The release does not have exactly the staged assets.');
  for (const asset of release.assets) {
    const uploaded = ready.assets.find(existing => existing.name === asset.name);
    if (!uploaded || uploaded.size !== asset.size || uploaded.digest !== asset.digest) throw new Error(`Incomplete upload: ${asset.name}`);
  }
  await assertTagTarget();
  const published = (await github.rest.repos.updateRelease({
    ...repository, release_id: draft.id, body: notes, draft: false, prerelease: false, make_latest: 'true',
  })).data;
  if (published.draft || published.tag_name !== release.tag) throw new Error('GitHub did not publish the requested version.');
  core.notice(`Published ${published.html_url}`);
};
