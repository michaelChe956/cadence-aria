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
              "max_code_context_chars": { "type": "integer" },
              "max_context_file_refs": { "type": "integer" },
              "max_traceability_refs": { "type": "integer" }
            }
          },
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

pub const WORK_ITEM_DRAFT_OUTPUT_SCHEMA: &str = r#"{
  "type": "object",
  "properties": {
    "draft": {
      "type": "object",
      "properties": {
        "outline_id": { "type": "string" },
        "logical_work_item_id": { "type": "string", "minLength": 1 },
        "canonical_contract": {
          "type": "object",
          "properties": {
            "schema_version": { "type": "integer", "enum": [1] },
            "identity": {
              "type": "object",
              "properties": {
                "logical_work_item_id": { "type": "string", "minLength": 1 },
                "title": { "type": "string" },
                "kind": {
                  "type": "string",
                  "enum": ["backend", "frontend", "integration", "e2e", "docs", "infra", "other"]
                }
              },
              "required": ["logical_work_item_id", "title", "kind"],
              "additionalProperties": false
            },
            "goal": {
              "type": "object",
              "properties": {
                "summary": { "type": "string" }
              },
              "required": ["summary"],
              "additionalProperties": false
            },
            "non_goals": {
              "type": "array",
              "items": { "type": "string" }
            },
            "input_contracts": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "contract_id": { "type": "string", "minLength": 1 },
                  "provider_logical_work_item_id": { "type": "string", "minLength": 1 },
                  "required_capabilities": {
                    "type": "array",
                    "items": { "type": "string" }
                  },
                  "compatibility_policy": {
                    "type": "string",
                    "enum": ["require_all", "require_any"]
                  }
                },
                "required": [
                  "contract_id",
                  "provider_logical_work_item_id",
                  "required_capabilities",
                  "compatibility_policy"
                ],
                "additionalProperties": false
              },
              "uniqueItems": true
            },
            "output_contracts": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "contract_id": { "type": "string", "minLength": 1 },
                  "capabilities": {
                    "type": "array",
                    "items": { "type": "string" }
                  }
                },
                "required": ["contract_id", "capabilities"],
                "additionalProperties": false
              },
              "uniqueItems": true
            },
            "tasks": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "task_id": { "type": "string", "minLength": 1 },
                  "statement": { "type": "string" },
                  "requirement_refs": {
                    "type": "array",
                    "items": { "type": "string" }
                  },
                  "done_when_refs": {
                    "type": "array",
                    "items": { "type": "string" }
                  }
                },
                "required": ["task_id", "statement", "requirement_refs", "done_when_refs"],
                "additionalProperties": false
              },
              "uniqueItems": true
            },
            "write_policy": {
              "type": "object",
              "properties": {
                "exclusive_scopes": {
                  "type": "array",
                  "items": { "type": "string" }
                },
                "forbidden_scopes": {
                  "type": "array",
                  "items": { "type": "string" }
                }
              },
              "required": ["exclusive_scopes", "forbidden_scopes"],
              "additionalProperties": false
            },
            "acceptance_criteria": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "criterion_id": { "type": "string", "minLength": 1 },
                  "statement": { "type": "string" },
                  "required_evidence": {
                    "type": "array",
                    "items": {
                      "type": "string",
                      "enum": [
                        "source_diff",
                        "non_zero_test_execution",
                        "manual_check",
                        "handoff_field"
                      ]
                    }
                  }
                },
                "required": ["criterion_id", "statement", "required_evidence"],
                "additionalProperties": false
              },
              "uniqueItems": true
            },
            "verification_checks": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "check_id": { "type": "string", "minLength": 1 },
                  "command": { "type": ["string", "null"] },
                  "manual_instruction": { "type": ["string", "null"] },
                  "required": { "type": "boolean" },
                  "non_zero_test_execution_required": { "type": "boolean" }
                },
                "required": [
                  "check_id",
                  "command",
                  "manual_instruction",
                  "required",
                  "non_zero_test_execution_required"
                ],
                "additionalProperties": false,
                "allOf": [{
                  "if": { "properties": { "required": { "const": true } } },
                  "then": {
                    "properties": { "command": { "type": "string", "minLength": 1 } },
                    "required": ["command"]
                  }
                }]
              },
              "uniqueItems": true
            },
            "handoff_contract": {
              "type": "object",
              "properties": {
                "required_fields": {
                  "type": "array",
                  "items": { "type": "string", "minLength": 1 },
                  "uniqueItems": true
                },
                "provided_contract_refs": {
                  "type": "array",
                  "items": { "type": "string", "minLength": 1 },
                  "uniqueItems": true
                },
                "reviewer_check_refs": {
                  "type": "array",
                  "items": { "type": "string", "minLength": 1 },
                  "uniqueItems": true
                }
              },
              "required": ["required_fields", "provided_contract_refs", "reviewer_check_refs"],
              "additionalProperties": false
            },
            "blocker_rules": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "reason_code": { "type": "string", "minLength": 1 },
                  "route": {
                    "type": "string",
                    "enum": [
                      "coder_rework",
                      "verification_retry",
                      "plan_repair_current",
                      "plan_repair_upstream",
                      "subgraph_replan",
                      "story_amendment",
                      "design_amendment",
                      "operational_gate"
                    ]
                  },
                  "target_contract_refs": {
                    "type": "array",
                    "items": { "type": "string" }
                  }
                },
                "required": ["reason_code", "route", "target_contract_refs"],
                "additionalProperties": false
              },
              "uniqueItems": true
            },
            "design_traceability": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "source_type": { "type": "string" },
                  "source_id": { "type": "string" },
                  "requirement_id": { "type": "string" }
                },
                "required": ["source_type", "source_id", "requirement_id"],
                "additionalProperties": false
              }
            }
          },
          "required": [
            "schema_version",
            "identity",
            "goal",
            "non_goals",
            "input_contracts",
            "output_contracts",
            "tasks",
            "write_policy",
            "acceptance_criteria",
            "verification_checks",
            "handoff_contract",
            "blocker_rules",
            "design_traceability"
          ],
          "additionalProperties": false
        },
        "verification_plan": {
          "type": "object",
          "properties": {
            "checks": {
              "type": "array",
              "items": {
                "type": "object",
                "properties": {
                  "check_id": { "type": "string", "minLength": 1 },
                  "command": { "type": ["string", "null"] },
                  "manual_instruction": { "type": ["string", "null"] },
                  "required": { "type": "boolean" },
                  "non_zero_test_execution_required": { "type": "boolean" }
                },
                "required": [
                  "check_id",
                  "command",
                  "manual_instruction",
                  "required",
                  "non_zero_test_execution_required"
                ],
                "additionalProperties": false,
                "allOf": [{
                  "if": { "properties": { "required": { "const": true } } },
                  "then": {
                    "properties": { "command": { "type": "string", "minLength": 1 } },
                    "required": ["command"]
                  }
                }]
              },
              "uniqueItems": true
            }
          },
          "required": ["checks"],
          "additionalProperties": false
        }
      },
      "required": ["outline_id", "logical_work_item_id", "canonical_contract", "verification_plan"],
      "additionalProperties": false
    }
  },
  "required": ["draft"],
  "additionalProperties": false
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
              "logical_work_item_id": { "type": "string" },
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
              "estimated_context_tokens": {
                "type": "integer",
                "minimum": 1,
                "maximum": 50000
              },
              "session_fit": {
                "type": "string",
                "enum": ["fits_single_agent_session"]
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
              "trusted_verification_commands": {
                "type": "array",
                "maxItems": 3,
                "items": {
                  "type": "object",
                  "properties": {
                    "command": { "type": "string", "maxLength": 48 },
                    "cwd": { "type": "string", "maxLength": 16 },
                    "purpose": { "type": "string", "maxLength": 32 },
                    "source_ref": { "type": "string", "maxLength": 32 }
                  },
                  "required": ["command", "cwd", "purpose", "source_ref"],
                  "additionalProperties": false
                }
              },
              "handoff_notes": { "type": "string" }
            },
            "required": [
              "outline_id",
              "logical_work_item_id",
              "title",
              "kind",
              "goal",
              "scope",
              "non_goals",
              "estimated_context_tokens",
              "session_fit",
              "source_story_spec_ids",
              "source_design_spec_ids",
              "exclusive_write_scopes",
              "forbidden_write_scopes",
              "depends_on",
              "verification_intent",
              "trusted_verification_commands",
              "handoff_notes"
            ],
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
