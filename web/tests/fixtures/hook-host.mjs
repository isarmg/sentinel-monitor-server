// Deterministic hook/unit host, not a browser or a full React renderer.
// Components and event handlers are loaded unchanged from their original TSX.
let current;
export function useState(initial) {
  const host = current, index = host.index++;
  if (!(index in host.slots)) {
    const slot = { value: typeof initial === 'function' ? initial() : initial };
    slot.set = next => {
      const value = typeof next === 'function' ? next(slot.value) : next;
      if (!Object.is(value, slot.value)) { slot.value = value; host.dirty = true; }
    };
    host.slots[index] = slot;
  }
  const slot = host.slots[index];
  return [slot.value, slot.set];
}
export function useRef(value) {
  const host = current, index = host.index++;
  return host.slots[index] ??= { current: value };
}
export function useMemo(factory, deps) {
  const memo = useRef(null);
  if (!memo.current || !deps || deps.some((dep, index) => !Object.is(dep, memo.current.deps[index])))
    memo.current = { deps, value: factory() };
  return memo.current.value;
}
export const StrictMode = ({ children }) => children;
export function useId() { return useRef(`unit-${current.index}`).current; }
export function useEffect(effect, deps) {
  const host = current, index = host.index++;
  const previous = host.slots[index];
  if (!previous || deps === undefined || deps.some((dep, i) => !Object.is(dep, previous.deps[i]))) {
    host.effects.push(() => {
      previous?.cleanup?.();
      host.slots[index] = { deps, cleanup: effect() };
    });
  }
}
export function jsx(type, props, key) { return { type, props: props ?? {}, key }; }
export const jsxs = jsx;
export const Fragment = 'fragment';
export class HookHost {
  slots = []; effects = []; dirty = true; index = 0;
  constructor(component, props = {}) { this.component = component; this.props = props; }
  render(beforeEffects) {
    for (let i = 0; i < 30; i++) {
      current = this; this.index = 0; this.effects = []; this.dirty = false;
      this.tree = this.component(this.props); current = undefined;
      beforeEffects?.(this.tree);
      const effects = this.effects; this.effects = [];
      for (const effect of effects) effect();
      if (!this.dirty) return this.tree;
    }
    throw new Error('State did not settle');
  }
  unmount() { for (const slot of this.slots) slot?.cleanup?.(); }
}
export function walk(node, predicate, result = []) {
  if (Array.isArray(node)) { for (const item of node) walk(item, predicate, result); return result; }
  if (!node || typeof node !== 'object') return result;
  if (predicate(node)) result.push(node);
  walk(node.props?.children, predicate, result);
  return result;
}
export function expand(node) {
  if (Array.isArray(node)) return node.map(expand);
  if (!node || typeof node !== 'object') return node;
  if (typeof node.type === 'function') return expand(node.type(node.props));
  return { ...node, props: { ...node.props, children: expand(node.props.children) } };
}
export function textContent(node) {
  if (Array.isArray(node)) return node.map(textContent).join(' ');
  if (node === null || node === undefined || typeof node === 'boolean') return '';
  if (typeof node !== 'object') return String(node);
  return textContent(node.props?.children);
}

export function useCallback(fn,deps){return useMemo(()=>fn,deps);}
