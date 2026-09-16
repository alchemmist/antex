//! Single-slot prompt stash behavior for the chat composer.

use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::PathBuf;

use super::user_messages::append_user_messages;
use super::user_messages::remap_colliding_paste_placeholders;
use super::*;

const PROMPT_STASH_VERSION: u32 = 1;
const PROMPT_STASH_MAX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(serde::Deserialize, serde::Serialize)]
struct PersistedPromptStash {
    version: u32,
    composer: ThreadComposerState,
}

impl ChatWidget {
    pub(super) fn toggle_prompt_stash(&mut self) {
        if let Some(stashed_composer) = self.stashed_composer.take() {
            let draft = self.bottom_pane.composer_draft_snapshot();
            let current_composer = ThreadComposerState {
                text: draft.text,
                text_elements: draft.text_elements,
                local_images: draft.local_images,
                remote_image_urls: draft.remote_image_urls,
                mention_bindings: draft.mention_bindings,
                pending_pastes: draft.pending_pastes,
            };

            let restored_composer = if current_composer.has_content() {
                let mut used_paste_placeholders = HashSet::new();
                let current_message = UserMessage {
                    text: current_composer.text,
                    text_elements: current_composer.text_elements,
                    local_images: current_composer.local_images,
                    remote_image_urls: current_composer.remote_image_urls,
                    mention_bindings: current_composer.mention_bindings,
                };
                let stashed_message = UserMessage {
                    text: stashed_composer.text,
                    text_elements: stashed_composer.text_elements,
                    local_images: stashed_composer.local_images,
                    remote_image_urls: stashed_composer.remote_image_urls,
                    mention_bindings: stashed_composer.mention_bindings,
                };
                let (current_message, mut pending_pastes) = remap_colliding_paste_placeholders(
                    current_message,
                    current_composer.pending_pastes,
                    &mut used_paste_placeholders,
                );
                let (stashed_message, stashed_pending_pastes) = remap_colliding_paste_placeholders(
                    stashed_message,
                    stashed_composer.pending_pastes,
                    &mut used_paste_placeholders,
                );
                pending_pastes.extend(stashed_pending_pastes);
                Self::composer_state_from_user_message(
                    append_user_messages(vec![current_message, stashed_message]),
                    pending_pastes,
                )
            } else {
                stashed_composer
            };

            self.restore_composer_state(restored_composer);
            self.bottom_pane.hide_prompt_stashed_indicator();
            if let Err(err) = self.remove_persisted_prompt_stash() {
                self.add_error_message(err);
            }
            self.request_redraw();
            return;
        }

        let draft = self.bottom_pane.composer_draft_snapshot();
        let composer = ThreadComposerState {
            text: draft.text,
            text_elements: draft.text_elements,
            local_images: draft.local_images,
            remote_image_urls: draft.remote_image_urls,
            mention_bindings: draft.mention_bindings,
            pending_pastes: draft.pending_pastes,
        };
        if !composer.has_content() {
            return;
        }

        self.stashed_composer = Some(composer);
        if self.thread_id.is_some()
            && let Err(err) = self.persist_prompt_stash()
        {
            self.add_error_message(err);
        }
        self.restore_composer_state(ThreadComposerState::default());
        self.bottom_pane.show_prompt_stashed_indicator();
        self.request_redraw();
    }

    fn prompt_stash_path(&self, thread_id: ThreadId) -> PathBuf {
        self.config
            .codex_home
            .join("prompt-stashes")
            .join(format!("{thread_id}.json"))
            .into_path_buf()
    }

    pub(super) fn persist_prompt_stash(&self) -> Result<(), String> {
        let thread_id = self
            .thread_id
            .ok_or_else(|| "Cannot persist prompt stash before the session starts.".to_string())?;
        let composer = self
            .stashed_composer
            .clone()
            .ok_or_else(|| "Cannot persist an empty prompt stash.".to_string())?;
        let encoded = serde_json::to_string(&PersistedPromptStash {
            version: PROMPT_STASH_VERSION,
            composer,
        })
        .map_err(|err| format!("Failed to encode prompt stash: {err}"))?;
        if encoded.len() as u64 > PROMPT_STASH_MAX_BYTES {
            return Err("Prompt stash is too large to persist safely.".to_string());
        }
        let path = self.prompt_stash_path(thread_id);
        antex_utils_path::write_atomically(&path, &encoded)
            .map_err(|err| format!("Failed to persist prompt stash: {err}"))
    }

    fn remove_persisted_prompt_stash(&self) -> Result<(), String> {
        let Some(thread_id) = self.thread_id else {
            return Ok(());
        };
        let path = self.prompt_stash_path(thread_id);
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(format!("Failed to clear persisted prompt stash: {err}")),
        }
    }

    pub(super) fn restore_persisted_prompt_stash(
        &mut self,
        thread_id: ThreadId,
    ) -> Result<(), String> {
        let path = self.prompt_stash_path(thread_id);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(format!("Failed to inspect prompt stash: {err}")),
        };
        if metadata.len() > PROMPT_STASH_MAX_BYTES {
            return Err("Persisted prompt stash is too large to restore safely.".to_string());
        }
        let encoded = std::fs::read_to_string(&path)
            .map_err(|err| format!("Failed to read prompt stash: {err}"))?;
        let persisted = serde_json::from_str::<PersistedPromptStash>(&encoded)
            .map_err(|err| format!("Failed to decode prompt stash: {err}"))?;
        if persisted.version != PROMPT_STASH_VERSION {
            return Err(format!(
                "Unsupported prompt stash version: {}",
                persisted.version
            ));
        }
        if !persisted.composer.has_content() {
            return Ok(());
        }
        self.stashed_composer = Some(persisted.composer);
        self.bottom_pane.show_prompt_stashed_indicator();
        Ok(())
    }
}

#[cfg(test)]
#[path = "prompt_stash_tests.rs"]
mod tests;
