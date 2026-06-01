import type { DocNode } from "../../ipc/apiDocs";

interface Props {
  nodes: DocNode[];
  selected: Set<string>;
  onChange: (next: Set<string>) => void;
  filter: string;
}

/** Collect all leaf hrefs in a subtree */
function collectLeaves(nodes: DocNode[]): string[] {
  const out: string[] = [];
  for (const n of nodes) {
    if (n.items.length === 0) {
      out.push(n.href);
    } else {
      out.push(...collectLeaves(n.items));
    }
  }
  return out;
}

/** True if this node or any descendant matches the filter */
function matchesFilter(node: DocNode, lc: string): boolean {
  if (node.title.toLowerCase().includes(lc)) return true;
  return node.items.some((child) => matchesFilter(child, lc));
}

function TreeNode({
  node,
  selected,
  onChange,
  filter,
  depth,
}: {
  node: DocNode;
  selected: Set<string>;
  onChange: (next: Set<string>) => void;
  filter: string;
  depth: number;
}) {
  const lc = filter.toLowerCase();
  if (lc && !matchesFilter(node, lc)) return null;

  const isLeaf = node.items.length === 0;

  if (isLeaf) {
    const checked = selected.has(node.href);
    return (
      <label
        className="doc-tree__leaf"
        style={{ paddingLeft: depth * 16 + 8 }}
      >
        <input
          type="checkbox"
          aria-label={node.title}
          checked={checked}
          onChange={() => {
            const next = new Set(selected);
            if (checked) next.delete(node.href);
            else next.add(node.href);
            onChange(next);
          }}
        />
        <span>{node.title}</span>
      </label>
    );
  }

  const leaves = collectLeaves(node.items);
  const allChecked = leaves.length > 0 && leaves.every((h) => selected.has(h));
  const someChecked = leaves.some((h) => selected.has(h));

  const toggleGroup = () => {
    const next = new Set(selected);
    if (allChecked) {
      leaves.forEach((h) => next.delete(h));
    } else {
      leaves.forEach((h) => next.add(h));
    }
    onChange(next);
  };

  return (
    <div className="doc-tree__group">
      <label
        className="doc-tree__group-header"
        style={{ paddingLeft: depth * 16 + 4 }}
      >
        <input
          type="checkbox"
          aria-label={node.title}
          checked={allChecked}
          ref={(el) => {
            if (el) el.indeterminate = someChecked && !allChecked;
          }}
          onChange={toggleGroup}
        />
        <span>{node.title}</span>
      </label>
      {node.items.map((child) => (
        <TreeNode
          key={child.href}
          node={child}
          selected={selected}
          onChange={onChange}
          filter={filter}
          depth={depth + 1}
        />
      ))}
    </div>
  );
}

export function DocTree({ nodes, selected, onChange, filter }: Props) {
  return (
    <div className="doc-tree">
      {nodes.map((node) => (
        <TreeNode
          key={node.href}
          node={node}
          selected={selected}
          onChange={onChange}
          filter={filter}
          depth={0}
        />
      ))}
    </div>
  );
}
