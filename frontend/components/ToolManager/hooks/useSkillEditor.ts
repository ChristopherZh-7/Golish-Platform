import { useCallback, useEffect, useRef, useState } from "react";
import {
  deleteSkill,
  listSkills,
  readSkill,
  type SkillFileInfo,
  writeSkill,
} from "@/lib/pentest/api";

interface UseSkillEditorOptions {
  toolName: string | null;
  skillsList: SkillFileInfo[];
  setSkillsList: (skills: SkillFileInfo[]) => void;
  skillDirty: boolean;
  setSkillDirty: (dirty: boolean) => void;
}

export function useSkillEditor(opts: UseSkillEditorOptions) {
  const { toolName, skillsList, setSkillsList, skillDirty, setSkillDirty } = opts;

  const [activeSkillId, setActiveSkillId] = useState<string | null>(null);
  const [skillContent, setSkillContent] = useState("");
  const [skillSaving, setSkillSaving] = useState(false);
  const [newSkillName, setNewSkillName] = useState("");
  const [showNewSkill, setShowNewSkill] = useState(false);
  // Tracks the last known on-disk content so the dirty flag is only set when
  // the user actually changes something. Milkdown's `markdownUpdated` listener
  // fires once on mount with a normalized version of the input markdown
  // (whitespace / trailing newlines may differ); without this baseline we
  // would falsely report "unsaved changes" the moment the editor mounts.
  const baselineContentRef = useRef<string>("");

  useEffect(() => {
    setActiveSkillId(null);
    setSkillContent("");
    setShowNewSkill(false);
    setNewSkillName("");
    setSkillDirty(false);
    baselineContentRef.current = "";
  }, [toolName, setSkillDirty]);

  const loadSkillContent = useCallback(
    async (skillId: string) => {
      if (!toolName) return;
      try {
        const content = await readSkill(toolName, skillId);
        baselineContentRef.current = content;
        setActiveSkillId(skillId);
        setSkillContent(content);
        setSkillDirty(false);
      } catch {
        baselineContentRef.current = "";
        setActiveSkillId(skillId);
        setSkillContent("");
        setSkillDirty(false);
      }
    },
    [toolName, setSkillDirty]
  );

  const handleSaveSkill = useCallback(async () => {
    if (!toolName || !activeSkillId) return;
    setSkillSaving(true);
    try {
      await writeSkill(toolName, activeSkillId, skillContent);
      baselineContentRef.current = skillContent;
      setSkillDirty(false);
    } catch (e) {
      console.error("[Skills] Save failed:", e);
    } finally {
      setSkillSaving(false);
    }
  }, [toolName, activeSkillId, skillContent, setSkillDirty]);

  const handleCreateSkill = useCallback(async () => {
    if (!toolName || !newSkillName.trim()) return;
    const id = newSkillName
      .trim()
      .toLowerCase()
      .replace(/\s+/g, "-")
      .replace(/[^a-z0-9-]/g, "");
    if (!id) return;
    const template = `# ${newSkillName.trim()}\n\n## Description\n\nDescribe what this skill does.\n\n## Usage\n\n\`\`\`bash\n${toolName} <args>\n\`\`\`\n\n## Notes\n\n- Add notes here\n`;
    try {
      await writeSkill(toolName, id, template);
      const updated = await listSkills(toolName);
      setSkillsList(updated);
      baselineContentRef.current = template;
      setActiveSkillId(id);
      setSkillContent(template);
      setSkillDirty(false);
      setNewSkillName("");
      setShowNewSkill(false);
    } catch (e) {
      console.error("[Skills] Create failed:", e);
    }
  }, [toolName, newSkillName, setSkillsList, setSkillDirty]);

  const handleDeleteSkill = useCallback(
    async (skillId: string) => {
      if (!toolName) return;
      try {
        await deleteSkill(toolName, skillId);
        const updated = await listSkills(toolName);
        setSkillsList(updated);
        if (activeSkillId === skillId) {
          baselineContentRef.current = "";
          setActiveSkillId(null);
          setSkillContent("");
          setSkillDirty(false);
        }
      } catch (e) {
        console.error("[Skills] Delete failed:", e);
      }
    },
    [toolName, activeSkillId, setSkillsList, setSkillDirty]
  );

  const updateContent = useCallback(
    (content: string) => {
      setSkillContent(content);
      setSkillDirty(content !== baselineContentRef.current);
    },
    [setSkillDirty]
  );

  return {
    activeSkillId,
    skillContent,
    skillSaving,
    newSkillName,
    setNewSkillName,
    showNewSkill,
    setShowNewSkill,
    loadSkillContent,
    handleSaveSkill,
    handleCreateSkill,
    handleDeleteSkill,
    updateContent,
    skillsList,
    skillDirty,
  };
}
