import { it, expect } from 'vitest';
import { en, ru, translate } from '../src/i18n';
it('both installer and launcher have complete non-empty translations', () => {
  expect(Object.keys(ru).sort()).toEqual(Object.keys(en).sort());
  for (const value of Object.values(ru)) expect(value.trim().length).toBeGreaterThan(0);
  expect(translate('en', 'installLanguage')).toBe('Choose your language');
  expect(translate('ru', 'play')).toBe('Играть');
});
