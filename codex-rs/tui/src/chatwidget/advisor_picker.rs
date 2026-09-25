//! Model and reasoning selection for the configured strategic advisor.

use super::*;

pub(super) const ADVISOR_MODEL_SELECTION_VIEW_ID: &str = "advisor-model-selection";

impl ChatWidget {
    pub(super) fn open_advisor_picker(&mut self) {
        if !self.is_session_configured() {
            self.add_info_message(
                "Advisor selection is disabled until startup completes.".to_string(),
                /*hint*/ None,
            );
            return;
        }
        let presets = self.model_catalog.try_list_models().unwrap_or_default();
        let request_id = uuid::Uuid::new_v4();
        self.model_popup_request_id = Some(request_id);
        self.open_advisor_picker_with_presets(presets);
        self.app_event_tx.send(AppEvent::FetchModels { request_id });
    }

    pub(super) fn open_advisor_picker_with_presets(&mut self, presets: Vec<ModelPreset>) {
        let current = self.config.advisor.model.as_deref();
        let model_ids = presets
            .iter()
            .filter(|preset| preset.show_in_picker && !Self::is_auto_model(&preset.model))
            .map(|preset| preset.model.clone())
            .collect();
        let items = presets
            .into_iter()
            .filter(|preset| preset.show_in_picker && !Self::is_auto_model(&preset.model))
            .map(|preset| {
                let selected = current == Some(preset.model.as_str());
                let name = preset.display_name.clone();
                let description =
                    (!preset.description.is_empty()).then_some(preset.description.clone());
                let model = preset.clone();
                SelectionItem {
                    name,
                    description,
                    is_current: selected,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::OpenAdvisorReasoningPopup {
                            model: model.clone(),
                        });
                    })],
                    dismiss_on_select: false,
                    dismiss_parent_on_child_accept: true,
                    ..Default::default()
                }
            })
            .collect::<Vec<_>>();

        if items.is_empty() {
            self.add_info_message(
                "No advisor models are available right now.".to_string(),
                /*hint*/ None,
            );
            return;
        }

        let header = self.model_menu_header("Select Advisor Model", "");
        self.show_model_selection_view(
            model_ids,
            SelectionViewParams {
                view_id: Some(ADVISOR_MODEL_SELECTION_VIEW_ID),
                title: Some("Advisor".to_string()),
                items,
                header,
                ..SelectionViewParams::picker()
            },
        );
    }

    pub(crate) fn open_advisor_reasoning_popup(&mut self, preset: ModelPreset) {
        let supported = &preset.supported_reasoning_efforts;
        let mut efforts = supported
            .iter()
            .map(|option| option.effort.clone())
            .collect::<Vec<_>>();
        if efforts.is_empty() {
            efforts.push(preset.default_reasoning_effort.clone());
        }
        let current_model = self.config.advisor.model.as_deref() == Some(preset.model.as_str());
        let current_effort = current_model
            .then(|| self.config.advisor.reasoning_effort.clone())
            .flatten();
        let model = preset.model.clone();
        let items = efforts
            .into_iter()
            .map(|effort| {
                let selected = current_model && current_effort.as_ref() == Some(&effort);
                let label = Self::reasoning_effort_label(&effort);
                let model = model.clone();
                SelectionItem {
                    name: label,
                    is_current: selected,
                    actions: vec![Box::new(move |tx| {
                        tx.send(AppEvent::PersistAdvisorSelection {
                            model: model.clone(),
                            effort: Some(effort.clone()),
                        });
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                }
            })
            .collect();
        let display_name = preset.display_name;
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(format!("Advisor Reasoning · {display_name}")),
            items,
            ..SelectionViewParams::picker()
        });
    }
}
