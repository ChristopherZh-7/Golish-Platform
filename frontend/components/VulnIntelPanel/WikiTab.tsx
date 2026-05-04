import type { VulnLink } from "./types";
import { useWikiTab } from "./useWikiTab";
import { WikiArticlePanel } from "./WikiArticlePanel";
import { WikiSidebar } from "./WikiSidebar";

export function WikiTab({
  link,
  cveId,
  onUpdateLink,
}: {
  link: VulnLink;
  cveId: string;
  onUpdateLink: (updater: (l: VulnLink) => VulnLink) => void;
}) {
  const state = useWikiTab(link, cveId, onUpdateLink);

  return (
    <div className="flex h-full min-h-0">
      <WikiSidebar
        link={link}
        selectedPath={state.selectedPath}
        setSelectedPath={state.setSelectedPath}
        fullTree={state.fullTree}
        loadingTree={state.loadingTree}
        expandedDirs={state.expandedDirs}
        toggleDir={state.toggleDir}
        linkedSet={state.linkedSet}
        linkedByCategory={state.linkedByCategory}
        suggestedPages={state.suggestedPages}
        searchQuery={state.searchQuery}
        setSearchQuery={state.setSearchQuery}
        searchResults={state.searchResults}
        searching={state.searching}
        browseAll={state.browseAll}
        setBrowseAll={state.setBrowseAll}
        adding={state.adding}
        setAdding={state.setAdding}
        newPath={state.newPath}
        setNewPath={state.setNewPath}
        creating={state.creating}
        setCreating={state.setCreating}
        createPath={state.createPath}
        setCreatePath={state.setCreatePath}
        handleLinkWiki={state.handleLinkWiki}
        handleUnlinkWiki={state.handleUnlinkWiki}
        handleCreatePage={state.handleCreatePage}
        navigateToWikiPage={state.navigateToWikiPage}
      />

      <div className="flex-1 min-h-0 flex flex-col overflow-hidden">
        <WikiArticlePanel
          link={link}
          selectedPath={state.selectedPath}
          articleContents={state.articleContents}
          editingPath={state.editingPath}
          editContent={state.editContent}
          setEditContent={state.setEditContent}
          setEditingPath={state.setEditingPath}
          isEditing={state.isEditing}
          selectedTitle={state.selectedTitle}
          selectedStatus={state.selectedStatus}
          selectedBody={state.selectedBody}
          selectedTags={state.selectedTags}
          linkedSet={state.linkedSet}
          backlinks={state.backlinks}
          handleLinkWiki={state.handleLinkWiki}
          handleUnlinkWiki={state.handleUnlinkWiki}
          handleStartEdit={state.handleStartEdit}
          handleSaveEdit={state.handleSaveEdit}
          handleDeletePage={state.handleDeletePage}
          navigateToWikiPage={state.navigateToWikiPage}
        />
      </div>
    </div>
  );
}
