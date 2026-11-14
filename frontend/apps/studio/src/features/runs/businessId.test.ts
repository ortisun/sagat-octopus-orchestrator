import { describe, expect, it } from 'vitest';
import { MAX_BUSINESS_ID, businessFields, businessIdProblem, readBusinessPrefix } from './businessId';

describe('the business identifier of a start', () => {
  it('needs none, and accepts 1 to 200 visible ASCII characters', () => {
    expect(businessIdProblem('')).toBeUndefined();
    for (const id of ['a', 'order-1001', 'a_b.c:d/e@f%g#h', '~!', 'x'.repeat(MAX_BUSINESS_ID)]) {
      expect(businessIdProblem(id), id).toBeUndefined();
    }
  });

  it('refuses a space, an accent, a control character and more than 200 characters, in words', () => {
    for (const id of ['has space', 'tab\there', 'ünï', 'new\nline', '日本', 'x'.repeat(MAX_BUSINESS_ID + 1)]) {
      expect(businessIdProblem(id), JSON.stringify(id)).toMatch(/characters|accent/);
    }
  });

  it('takes another ceiling, for the prefix of a schedule, and says it', () => {
    expect(businessIdProblem('x'.repeat(160), 160)).toBeUndefined();
    expect(businessIdProblem('x'.repeat(161), 160)).toBe('At most 160 characters.');
    expect(businessIdProblem('x'.repeat(161))).toBeUndefined();
    expect(businessIdProblem('has space', 160)).toMatch(/no space/);
  });

  it('sends nothing without an identifier, and no default policy with one', () => {
    expect(businessFields('', 'USE_EXISTING', 'REJECT_DUPLICATE')).toEqual({});
    expect(businessFields('order-1', 'FAIL', 'ALLOW_DUPLICATE')).toEqual({ businessId: 'order-1' });
  });

  it('sends the policies that are not the defaults', () => {
    expect(businessFields('order-1', 'USE_EXISTING', 'ALLOW_DUPLICATE')).toEqual({ businessId: 'order-1', businessIdConflictPolicy: 'USE_EXISTING' });
    expect(businessFields('order-1', 'FAIL', 'REJECT_DUPLICATE')).toEqual({ businessId: 'order-1', businessIdReusePolicy: 'REJECT_DUPLICATE' });
    expect(businessFields('order-1', 'USE_EXISTING', 'ALLOW_DUPLICATE_FAILED_ONLY')).toEqual({
      businessId: 'order-1',
      businessIdConflictPolicy: 'USE_EXISTING',
      businessIdReusePolicy: 'ALLOW_DUPLICATE_FAILED_ONLY',
    });
  });

  it('reads from the address only what could be a prefix', () => {
    expect(readBusinessPrefix(null)).toBe('');
    expect(readBusinessPrefix('order-')).toBe('order-');
    expect(readBusinessPrefix('has space')).toBe('');
    expect(readBusinessPrefix('x'.repeat(MAX_BUSINESS_ID + 1))).toBe('');
  });
});
