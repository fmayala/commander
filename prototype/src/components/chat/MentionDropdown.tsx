import type { MentionOption } from "../../types";
import styles from "./MentionDropdown.module.css";

interface Props {
  options: MentionOption[];
  selectedIndex: number;
  onSelect: (option: MentionOption) => void;
}

// rendering-hoist-jsx / js-hoist-regexp: hoist static data outside component
const GROUP_ORDER: MentionOption["kind"][] = ["repo", "agent", "task", "file"];
const GROUP_LABELS: Record<string, string> = {
  repo: "Repos",
  agent: "Agents",
  task: "Tasks",
  file: "Files",
};

export function MentionDropdown({ options, selectedIndex, onSelect }: Props) {
  if (options.length === 0) return null;

  const grouped = GROUP_ORDER
    .map((kind) => ({
      kind,
      items: options.filter((o) => o.kind === kind),
    }))
    .filter((g) => g.items.length > 0);

  let flatIndex = 0;

  return (
    <div className={styles.dropdown}>
      {grouped.map((group) => (
        <div key={group.kind}>
          <div className={styles.groupLabel}>{GROUP_LABELS[group.kind]}</div>
          {group.items.map((option) => {
            const idx = flatIndex++;
            return (
              <div
                key={option.id}
                className={styles.item}
                data-selected={idx === selectedIndex}
                onClick={() => onSelect(option)}
              >
                <span className={styles.kindBadge}>{option.kind}</span>
                <span className={styles.itemLabel}>{option.label}</span>
                {option.secondary && (
                  <span className={styles.itemSecondary}>{option.secondary}</span>
                )}
              </div>
            );
          })}
        </div>
      ))}
    </div>
  );
}
