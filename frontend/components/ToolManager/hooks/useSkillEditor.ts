import { useCallback, useState } from "react";
import {
  deleteSkill,
  listSkills,
  readSkill,
  writeSkill,
  type SkillFileInfo,
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

  const loadSkillContent = useCallback(
    async (skillId: string) => {
      if (!toolName) return;
      try {
        const content = await readSkill(toolName, skillId);
        setActiveSkillId(skillId);
        setSkillContent(content);
        setSkillDirty(false);
      } catch {
        setActiveSkillId(skillId);
        setSkillContent("");
        setSkillDirty(false);
      }
    },
    [toolName, setSkillDirty],
  );

  const handleSaveSkill = useCallback(async () => {
    if (!toolName || !activeSkillId) return;
    setSkillSaving(true);
    try {
      await writeSkill(toolName, activeSkillId, skillContent);
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
          setActiveSkillId(null);
          setSkillContent("");
          setSkillDirty(false);
        }
      } catch (e) {
        console.error("[Skills] Delete failed:", e);
      }
    },
    [toolName, activeSkillId, setSkillsList, setSkillDirty],
  );

  const updateContent = useCallback(
    (content: string) => {
      setSkillContent(content);
      setSkillDirty(true);
    },
    [setSkillDirty],
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
