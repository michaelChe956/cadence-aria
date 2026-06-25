pub(crate) const WORK_ITEM_SPLIT_OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "repository_profile": {
      "type": "object",
      "properties": {
        "confidence": { "type": "string" },
        "detected_layers": { "type": "array", "items": { "type": "string" } },
        "split_recommendation": { "type": "string" },
        "languages": { "type": "array", "items": { "type": "string" } },
        "frameworks": { "type": "array", "items": { "type": "string" } },
        "package_managers": { "type": "array", "items": { "type": "string" } },
        "test_frameworks": { "type": "array", "items": { "type": "string" } },
        "build_systems": { "type": "array", "items": { "type": "string" } },
        "verification_capabilities": { "type": "array", "items": { "type": "string" } },
        "uncertainties": { "type": "array", "items": { "type": "string" } }
      },
      "required": ["confidence", "detected_layers", "split_recommendation"]
    },
    "plan": {
      "type": "object",
      "properties": {
        "work_item_ids": { "type": "array", "items": { "type": "string" } },
        "dependency_graph": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "from_work_item_id": { "type": "string" },
              "to_work_item_id": { "type": "string" }
            },
            "required": ["from_work_item_id", "to_work_item_id"]
          }
        }
      }
    },
    "work_items": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": { "type": "string" },
          "kind": { "type": "string" },
          "sequence_hint": { "type": "integer" },
          "depends_on": { "type": "array", "items": { "type": "integer" } },
          "exclusive_write_scopes": { "type": "array", "items": { "type": "string" } },
          "forbidden_write_scopes": { "type": "array", "items": { "type": "string" } },
          "context_budget": {
            "type": "object",
            "properties": {
              "target_context_k": { "type": "string" },
              "max_summary_chars": { "type": "integer" },
              "max_handoff_chars": { "type": "integer" },
              "max_code_context_chars": { "type": "integer" },
              "max_context_file_refs": { "type": "integer" },
              "max_traceability_refs": { "type": "integer" },
              "max_dependency_handoffs": { "type": "integer" }
            }
          },
          "required_handoff_from": { "type": "array", "items": { "type": "string" } },
          "require_execution_plan_confirm": { "type": "boolean" }
        },
        "required": ["title", "kind"]
      }
    },
    "verification_plans": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "scope": { "type": "string" },
          "commands": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "id": { "type": "string" },
                "label": { "type": "string" },
                "command": { "type": "string" },
                "cwd": { "type": "string" },
                "purpose": { "type": "string" },
                "required": { "type": "boolean" },
                "timeout_seconds": { "type": "integer" },
                "safety": { "type": "string" }
              },
              "required": ["label", "command", "purpose"]
            }
          },
          "manual_checks": {
            "type": "array",
            "items": {
              "type": "object",
              "properties": {
                "id": { "type": "string" },
                "label": { "type": "string" },
                "instructions": { "type": "string" },
                "required": { "type": "boolean" }
              },
              "required": ["label", "instructions"]
            }
          },
          "required_gates": { "type": "array", "items": { "type": "string" } },
          "risk_notes": { "type": "array", "items": { "type": "string" } },
          "confidence": { "type": "string" },
          "fallback_policy": { "type": "string" }
        }
      }
    }
  },
  "required": ["repository_profile", "work_items", "verification_plans"]
}"#;

pub(crate) const WORK_ITEM_DRAFT_OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "draft": {
      "type": "object",
      "properties": {
        "outline_id": { "type": "string" },
        "title": { "type": "string" },
        "kind": { "type": "string" },
        "goal": { "type": "string" },
        "implementation_context": { "type": "string" },
        "exclusive_write_scopes": {
          "type": "array",
          "items": { "type": "string" }
        },
        "forbidden_write_scopes": {
          "type": "array",
          "items": { "type": "string" }
        },
        "depends_on_outline_ids": {
          "type": "array",
          "items": { "type": "string" }
        },
        "required_handoff_from_outline_ids": {
          "type": "array",
          "items": { "type": "string" }
        },
        "handoff_summary": { "type": "string" },
        "verification_plan": {
          "type": "object",
          "properties": {
            "commands": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "id": { "type": "string" },
                  "label": { "type": "string" },
                  "command": { "type": "string" },
                  "cwd": { "type": "string" },
                  "purpose": { "type": "string" },
                  "required": { "type": "boolean" },
                  "timeout_seconds": { "type": "integer" },
                  "safety": { "type": "string" }
                },
                "required": ["id", "label", "command", "purpose"]
              }
            },
            "manual_checks": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "id": { "type": "string" },
                  "label": { "type": "string" },
                  "instructions": { "type": "string" },
                  "required": { "type": "boolean" }
                },
                "required": ["id", "label", "instructions"]
              }
            },
            "required_gates": {
              "type": "array",
              "items": { "type": "string" }
            }
          },
          "required": ["commands", "manual_checks", "required_gates"]
        }
      },
      "required": [
        "outline_id",
        "title",
        "kind",
        "goal",
        "implementation_context",
        "exclusive_write_scopes",
        "forbidden_write_scopes",
        "depends_on_outline_ids",
        "required_handoff_from_outline_ids",
        "handoff_summary",
        "verification_plan"
      ]
    }
  },
  "required": ["draft"]
}"#;

pub(crate) const WORK_ITEM_PLAN_OUTLINE_OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "outline": {
      "type": "object",
      "properties": {
        "id": { "type": "string" },
        "project_id": { "type": "string" },
        "issue_id": { "type": "string" },
        "source_story_spec_ids": {
          "type": "array",
          "items": { "type": "string" }
        },
        "source_design_spec_ids": {
          "type": "array",
          "items": { "type": "string" }
        },
        "strategy_summary": { "type": "string" },
        "work_item_outlines": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "outline_id": { "type": "string" },
              "title": { "type": "string" },
              "kind": {
                "type": "string",
                "enum": ["backend", "frontend", "integration", "e2e", "docs", "infra", "other"]
              },
              "goal": { "type": "string" },
              "scope": {
                "type": "array",
                "items": { "type": "string" }
              },
              "non_goals": {
                "type": "array",
                "items": { "type": "string" }
              },
              "source_story_spec_ids": {
                "type": "array",
                "items": { "type": "string" }
              },
              "source_design_spec_ids": {
                "type": "array",
                "items": { "type": "string" }
              },
              "exclusive_write_scopes": {
                "type": "array",
                "items": { "type": "string" }
              },
              "forbidden_write_scopes": {
                "type": "array",
                "items": { "type": "string" }
              },
              "depends_on": {
                "type": "array",
                "items": { "type": "string" }
              },
              "verification_intent": {
                "type": "array",
                "items": { "type": "string" }
              },
              "handoff_notes": { "type": "string" }
            },
            "required": [
              "outline_id",
              "title",
              "kind",
              "goal",
              "scope",
              "non_goals",
              "source_story_spec_ids",
              "source_design_spec_ids",
              "exclusive_write_scopes",
              "forbidden_write_scopes",
              "depends_on",
              "verification_intent",
              "handoff_notes"
            ],
            "additionalProperties": false
          }
        },
        "dependency_graph": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "from_outline_id": { "type": "string" },
              "to_outline_id": { "type": "string" }
            },
            "required": ["from_outline_id", "to_outline_id"],
            "additionalProperties": false
          }
        },
        "risks": {
          "type": "array",
          "items": { "type": "string" }
        },
        "handoff_strategy": { "type": "string" },
        "status": { "type": "string" }
      },
      "required": [
        "id",
        "project_id",
        "issue_id",
        "source_story_spec_ids",
        "source_design_spec_ids",
        "strategy_summary",
        "work_item_outlines",
        "dependency_graph",
        "risks",
        "handoff_strategy",
        "status"
      ],
      "additionalProperties": false
    },
    "context_blockers": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "code": { "type": "string" },
          "message": { "type": "string" },
          "needed_context": {
            "type": "array",
            "items": { "type": "string" }
          }
        },
        "required": ["code", "message", "needed_context"],
        "additionalProperties": false
      }
    }
  },
  "oneOf": [
    {
      "required": ["outline"],
      "properties": {
        "context_blockers": { "maxItems": 0 }
      }
    },
    {
      "required": ["context_blockers"],
      "properties": {
        "context_blockers": { "minItems": 1 }
      },
      "not": { "required": ["outline"] }
    }
  ],
  "additionalProperties": false
}"#;
