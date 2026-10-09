import assert from 'node:assert/strict';
import { test } from 'node:test';
import { displayLabel } from '../src/display-labels.ts';

test('unknown event labels cannot resolve inherited object properties', () => {
  assert.equal(displayLabel('camera.online'), 'Camera is back online');
  assert.equal(displayLabel('client.authorization.rotate'), 'Change password');
  for (const value of ['__proto__', 'constructor', 'toString']) assert.equal(displayLabel(value), 'Unrecognized event');
});
