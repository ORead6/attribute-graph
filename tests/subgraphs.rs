//! Subgraph contracts expressed as SwiftUI-style consumer scenarios.
//!
//! Names such as `settings_view` and `detail_sheet` show how a higher UI layer
//! could group attribute nodes by view lifetime. The attribute graph itself
//! stores values, dependencies, caches, and ownership; it does not create
//! SwiftUI views or decide their identity.

use std::any::type_name;
use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use attribute_graph::{
    AttributeGraph, Edge, EvaluationContext, GraphError, NodeId, NodeState, RuleDescriptor,
    RuleHandle, SubgraphId, TypeDescriptor, UpdateFn, ValueStorage,
};

const VIEW_METRIC: TypeDescriptor = TypeDescriptor::new("i64");

struct FixedViewMetricRule {
    points: i64,
}

struct ForwardViewMetricRule {
    source_metric: NodeId,
}

struct ConditionalContentMetricRule {
    is_presented: NodeId,
    presented_metric: NodeId,
    placeholder_metric: NodeId,
}

struct ViewLifetimeRule {
    destroy_count: Rc<Cell<usize>>,
    points: i64,
}

impl Drop for ViewLifetimeRule {
    fn drop(&mut self) {
        self.destroy_count.set(self.destroy_count.get() + 1);
    }
}

fn boxed_rule<T: 'static>(
    body: T,
    update: UpdateFn,
    value_type: TypeDescriptor,
    debug_name: &'static str,
) -> RuleDescriptor {
    let body = Box::new(body);
    let handle = RuleHandle::from_raw(Box::into_raw(body) as usize);

    RuleDescriptor::new(
        handle,
        update,
        TypeDescriptor::new(type_name::<T>()),
        value_type,
        debug_name,
    )
    .with_destroy(drop_boxed_rule::<T>)
}

fn drop_boxed_rule<T>(handle: RuleHandle) {
    unsafe {
        drop(Box::from_raw(handle.raw() as *mut T));
    }
}

fn rule_body<T>(handle: RuleHandle) -> &'static T {
    unsafe { &*(handle.raw() as *const T) }
}

fn update_fixed_view_metric(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<FixedViewMetricRule>(handle);
    context.set_output(ValueStorage::from_i64(rule.points));
    Ok(())
}

fn update_forwarded_view_metric(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<ForwardViewMetricRule>(handle);
    let value = context.read(rule.source_metric)?;
    context.set_output(value);
    Ok(())
}

fn update_conditional_content_metric(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<ConditionalContentMetricRule>(handle);
    let is_presented = context
        .read(rule.is_presented)?
        .as_bool()
        .expect("SwiftUI-style presentation state should be a bool");
    let value = context.read(if is_presented {
        rule.presented_metric
    } else {
        rule.placeholder_metric
    })?;
    context.set_output(value);
    Ok(())
}

fn update_view_lifetime_metric(
    handle: RuleHandle,
    context: &mut EvaluationContext<'_>,
) -> Result<(), GraphError> {
    let rule = rule_body::<ViewLifetimeRule>(handle);
    context.set_output(ValueStorage::from_i64(rule.points));
    Ok(())
}

fn add_view_metric(graph: &mut AttributeGraph, view_scope: SubgraphId, points: i64) -> NodeId {
    graph
        .with_subgraph(view_scope, |graph| {
            graph.add_source(ValueStorage::from_i64(points))
        })
        .expect("SwiftUI-style view scope should be enterable")
}

#[test]
fn a_swiftui_view_scope_owns_every_attribute_created_while_its_builder_is_active() {
    let mut graph = AttributeGraph::new();
    let app_safe_area_inset = graph.add_source(ValueStorage::from_i64(0));
    let profile_card = graph.create_subgraph(None).unwrap();

    assert_eq!(graph.current_subgraph(), None);
    assert!(graph.contains_subgraph(profile_card));
    assert!(graph.subgraph(profile_card).is_some());
    assert_eq!(graph.subgraph_count(), 1);
    assert_eq!(graph.subgraphs(), vec![profile_card]);
    assert_eq!(graph.subgraph(profile_card).unwrap().id(), profile_card);
    assert_eq!(graph.subgraph(profile_card).unwrap().parent(), None);
    assert_eq!(graph.subgraph(profile_card).unwrap().children(), vec![]);
    assert_eq!(graph.subgraph(profile_card).unwrap().nodes(), vec![]);

    let (raw_padding, raw_corner_radius, typed_line_limit, typed_row_height) = graph
        .with_subgraph(profile_card, |graph| {
            assert_eq!(graph.current_subgraph(), Some(profile_card));

            let raw_padding = graph.add_source(ValueStorage::from_i64(1));
            let raw_corner_radius = graph.add_derived(boxed_rule(
                FixedViewMetricRule { points: 2 },
                update_fixed_view_metric,
                VIEW_METRIC,
                "ProfileCard.cornerRadius",
            ));
            let typed_line_limit = graph.add_static_attribute(3_i64);
            let typed_row_height = graph
                .add_dynamic_attribute::<i64>(boxed_rule(
                    FixedViewMetricRule { points: 4 },
                    update_fixed_view_metric,
                    VIEW_METRIC,
                    "ProfileCard.rowHeight",
                ))
                .unwrap();

            (
                raw_padding,
                raw_corner_radius,
                typed_line_limit.id(),
                typed_row_height.id(),
            )
        })
        .unwrap();

    assert_eq!(graph.current_subgraph(), None);
    assert_eq!(graph.node(app_safe_area_inset).unwrap().subgraph_id(), None);
    for attribute in [
        raw_padding,
        raw_corner_radius,
        typed_line_limit,
        typed_row_height,
    ] {
        assert_eq!(
            graph.node(attribute).unwrap().subgraph_id(),
            Some(profile_card)
        );
    }
    assert_eq!(
        graph.subgraph(profile_card).unwrap().nodes(),
        vec![
            raw_padding,
            raw_corner_radius,
            typed_line_limit,
            typed_row_height
        ]
    );

    let app_dynamic_type_size = graph.add_source(ValueStorage::from_i64(5));
    assert_eq!(
        graph.node(app_dynamic_type_size).unwrap().subgraph_id(),
        None
    );
}

#[test]
fn building_a_swiftui_view_returns_its_scope_and_keeps_layout_attributes_together() {
    let mut graph = AttributeGraph::new();

    let (settings_view, (available_width, content_width)) = graph
        .build_subgraph(None, |graph, building_settings_view| {
            assert_eq!(graph.current_subgraph(), Some(building_settings_view));
            let available_width = graph.add_source(ValueStorage::from_i64(9));
            let content_width = graph.add_derived(boxed_rule(
                ForwardViewMetricRule {
                    source_metric: available_width,
                },
                update_forwarded_view_metric,
                VIEW_METRIC,
                "SettingsView.contentWidth",
            ));
            Ok((available_width, content_width))
        })
        .unwrap();

    assert_eq!(graph.current_subgraph(), None);
    assert_eq!(
        graph.node(available_width).unwrap().subgraph_id(),
        Some(settings_view)
    );
    assert_eq!(
        graph.node(content_width).unwrap().subgraph_id(),
        Some(settings_view)
    );
    assert_eq!(graph.read_value(content_width).unwrap().as_i64(), Some(9));
}

#[test]
fn nested_swiftui_builders_assign_to_the_innermost_view_and_restore_the_parent() {
    let mut graph = AttributeGraph::new();
    let navigation_stack = graph.create_subgraph(None).unwrap();
    let detail_sheet = graph.create_subgraph(Some(navigation_stack)).unwrap();
    let retained_sheet_metric_after_error = Cell::new(None);

    assert_eq!(
        graph.subgraph(navigation_stack).unwrap().children(),
        vec![detail_sheet]
    );
    assert_eq!(
        graph.subgraph(detail_sheet).unwrap().parent(),
        Some(navigation_stack)
    );

    let (navigation_title_height, sheet_detent_height, toolbar_height) = graph
        .with_subgraph(navigation_stack, |graph| {
            assert_eq!(graph.current_subgraph(), Some(navigation_stack));
            let navigation_title_height = graph.add_source(ValueStorage::from_i64(1));

            let sheet_detent_height = graph
                .with_subgraph(detail_sheet, |graph| {
                    assert_eq!(graph.current_subgraph(), Some(detail_sheet));
                    graph.add_source(ValueStorage::from_i64(2))
                })
                .unwrap();
            assert_eq!(graph.current_subgraph(), Some(navigation_stack));

            let sheet_builder_error = graph
                .with_subgraph(detail_sheet, |graph| -> Result<(), GraphError> {
                    retained_sheet_metric_after_error
                        .set(Some(graph.add_source(ValueStorage::from_i64(20))));
                    Err(GraphError::CycleDetected)
                })
                .unwrap();
            assert_eq!(sheet_builder_error, Err(GraphError::CycleDetected));
            assert_eq!(graph.current_subgraph(), Some(navigation_stack));

            let sheet_builder_panic = catch_unwind(AssertUnwindSafe(|| {
                let _ = graph.with_subgraph(detail_sheet, |_graph| -> () {
                    panic!("intentional SwiftUI-style sheet construction panic")
                });
            }));
            assert!(sheet_builder_panic.is_err());
            assert_eq!(graph.current_subgraph(), Some(navigation_stack));

            let toolbar_height = graph.add_source(ValueStorage::from_i64(3));
            (navigation_title_height, sheet_detent_height, toolbar_height)
        })
        .unwrap();

    assert_eq!(graph.current_subgraph(), None);
    assert_eq!(
        graph.node(navigation_title_height).unwrap().subgraph_id(),
        Some(navigation_stack)
    );
    assert_eq!(
        graph.node(sheet_detent_height).unwrap().subgraph_id(),
        Some(detail_sheet)
    );
    assert_eq!(
        graph
            .node(retained_sheet_metric_after_error.get().unwrap())
            .unwrap()
            .subgraph_id(),
        Some(detail_sheet),
        "with_subgraph restores the parent view context but does not roll back attributes",
    );
    assert_eq!(
        graph.node(toolbar_height).unwrap().subgraph_id(),
        Some(navigation_stack)
    );
}

#[test]
fn a_failed_swiftui_screen_build_rolls_back_its_view_tree_and_never_reuses_ids() {
    let mut graph = AttributeGraph::new();
    let app_dynamic_type_scale = graph.add_source(ValueStorage::from_i64(100));
    let failed_screen = Cell::new(None);
    let failed_overlay = Cell::new(None);
    let failed_layout_metric = Cell::new(None);
    let destroy_count = Rc::new(Cell::new(0));

    let result: Result<(SubgraphId, ()), GraphError> =
        graph.build_subgraph(None, |graph, building_screen| {
            failed_screen.set(Some(building_screen));
            let screen_width = graph.add_derived(boxed_rule(
                ViewLifetimeRule {
                    destroy_count: Rc::clone(&destroy_count),
                    points: 1,
                },
                update_view_lifetime_metric,
                VIEW_METRIC,
                "FailedScreen.width",
            ));
            failed_layout_metric.set(Some(screen_width));

            let overlay = graph.create_subgraph(Some(building_screen))?;
            failed_overlay.set(Some(overlay));
            graph.with_subgraph(overlay, |graph| {
                graph.add_source(ValueStorage::from_i64(2));
            })?;

            Err(GraphError::CycleDetected)
        });

    assert_eq!(result, Err(GraphError::CycleDetected));
    assert_eq!(graph.current_subgraph(), None);
    assert_eq!(graph.subgraph_count(), 0);
    assert_eq!(graph.node_count(), 1);
    assert!(graph.contains_node(app_dynamic_type_scale));
    assert!(!graph.contains_node(failed_layout_metric.get().unwrap()));
    assert!(!graph.contains_subgraph(failed_screen.get().unwrap()));
    assert!(!graph.contains_subgraph(failed_overlay.get().unwrap()));
    assert_eq!(destroy_count.get(), 1);

    let replacement_screen = graph.create_subgraph(None).unwrap();
    assert_ne!(replacement_screen, failed_screen.get().unwrap());
    assert_ne!(replacement_screen, failed_overlay.get().unwrap());
    let replacement_screen_width = add_view_metric(&mut graph, replacement_screen, 3);
    assert_ne!(
        replacement_screen_width,
        failed_layout_metric.get().unwrap()
    );
}

#[test]
fn a_panicking_sheet_build_rolls_back_and_restores_the_dashboard_builder() {
    let mut graph = AttributeGraph::new();
    let dashboard_view = graph.create_subgraph(None).unwrap();
    let failed_sheet = Cell::new(None);
    let failed_sheet_metric = Cell::new(None);
    let destroy_count = Rc::new(Cell::new(0));

    let dashboard_width = graph
        .with_subgraph(dashboard_view, |graph| {
            let sheet_build_panic = catch_unwind(AssertUnwindSafe(|| {
                let _ = graph.build_subgraph(
                    Some(dashboard_view),
                    |graph, building_sheet| -> Result<(), GraphError> {
                        failed_sheet.set(Some(building_sheet));
                        let sheet_height = graph.add_derived(boxed_rule(
                            ViewLifetimeRule {
                                destroy_count: Rc::clone(&destroy_count),
                                points: 1,
                            },
                            update_view_lifetime_metric,
                            VIEW_METRIC,
                            "FailedSheet.height",
                        ));
                        failed_sheet_metric.set(Some(sheet_height));
                        panic!("intentional SwiftUI-style sheet build panic")
                    },
                );
            }));

            assert!(sheet_build_panic.is_err());
            assert_eq!(graph.current_subgraph(), Some(dashboard_view));
            graph.add_source(ValueStorage::from_i64(2))
        })
        .unwrap();

    assert_eq!(graph.current_subgraph(), None);
    assert_eq!(graph.subgraph_count(), 1);
    assert!(graph.contains_subgraph(dashboard_view));
    assert_eq!(
        graph.node(dashboard_width).unwrap().subgraph_id(),
        Some(dashboard_view)
    );
    assert!(!graph.contains_subgraph(failed_sheet.get().unwrap()));
    assert!(!graph.contains_node(failed_sheet_metric.get().unwrap()));
    assert_eq!(destroy_count.get(), 1);
}

#[test]
fn removing_a_swiftui_row_also_removes_its_accessory_but_keeps_its_section_and_sibling() {
    let mut graph = AttributeGraph::new();
    let settings_section = graph.create_subgraph(None).unwrap();
    let account_row = graph.create_subgraph(Some(settings_section)).unwrap();
    let disclosure_accessory = graph.create_subgraph(Some(account_row)).unwrap();
    let notifications_row = graph.create_subgraph(Some(settings_section)).unwrap();

    let section_spacing = add_view_metric(&mut graph, settings_section, 1);
    let account_row_height = add_view_metric(&mut graph, account_row, 2);
    let disclosure_width = add_view_metric(&mut graph, disclosure_accessory, 3);
    let notifications_row_height = add_view_metric(&mut graph, notifications_row, 4);

    let removed_account_row = graph.remove_subgraph(account_row).unwrap();
    assert!(graph.contains_subgraph(settings_section));
    assert!(!graph.contains_subgraph(account_row));
    assert!(!graph.contains_subgraph(disclosure_accessory));
    assert!(graph.contains_subgraph(notifications_row));
    assert_eq!(graph.subgraph_count(), 2);
    assert!(graph.contains_node(section_spacing));
    assert!(!graph.contains_node(account_row_height));
    assert!(!graph.contains_node(disclosure_width));
    assert!(graph.contains_node(notifications_row_height));
    assert_eq!(
        removed_account_row.subgraphs,
        vec![account_row, disclosure_accessory]
    );
    assert_eq!(
        removed_account_row.nodes,
        vec![account_row_height, disclosure_width]
    );
    assert_eq!(removed_account_row.dirtied_dependents, vec![]);
    drop(removed_account_row);

    let removed_settings_section = graph.remove_subgraph(settings_section).unwrap();
    assert!(!graph.contains_subgraph(settings_section));
    assert!(!graph.contains_subgraph(notifications_row));
    assert_eq!(graph.subgraph_count(), 0);
    assert!(!graph.contains_node(section_spacing));
    assert!(!graph.contains_node(notifications_row_height));
    assert_eq!(
        removed_settings_section.subgraphs,
        vec![settings_section, notifications_row]
    );
    assert_eq!(
        removed_settings_section.nodes,
        vec![section_spacing, notifications_row_height]
    );
    assert_eq!(removed_settings_section.dirtied_dependents, vec![]);
    drop(removed_settings_section);
}

#[test]
fn removing_a_child_view_invalidates_container_and_host_layout_without_removing_them() {
    let mut graph = AttributeGraph::new();
    let profile_screen = graph.create_subgraph(None).unwrap();
    let avatar_view = graph.create_subgraph(Some(profile_screen)).unwrap();
    let avatar_size_preference = add_view_metric(&mut graph, avatar_view, 8);
    let profile_layout_preference = graph
        .with_subgraph(profile_screen, |graph| {
            graph.add_derived(boxed_rule(
                ForwardViewMetricRule {
                    source_metric: avatar_size_preference,
                },
                update_forwarded_view_metric,
                VIEW_METRIC,
                "ProfileScreen.layoutPreference",
            ))
        })
        .unwrap();
    let hosting_view_layout = graph.add_derived(boxed_rule(
        ForwardViewMetricRule {
            source_metric: profile_layout_preference,
        },
        update_forwarded_view_metric,
        VIEW_METRIC,
        "UIHostingView.layoutPreference",
    ));

    assert_eq!(
        graph.read_value(hosting_view_layout).unwrap().as_i64(),
        Some(8)
    );

    let removal = graph.remove_subgraph(avatar_view).unwrap();
    assert_eq!(removal.subgraphs, vec![avatar_view]);
    assert_eq!(removal.nodes, vec![avatar_size_preference]);
    assert_eq!(removal.dirtied_dependents, vec![profile_layout_preference]);
    assert!(graph.contains_subgraph(profile_screen));
    assert!(!graph.contains_subgraph(avatar_view));
    assert!(graph.contains_node(profile_layout_preference));
    assert!(graph.contains_node(hosting_view_layout));
    assert_eq!(
        graph.node(profile_layout_preference).unwrap().state(),
        NodeState::Dirty
    );
    assert_eq!(
        graph.node(hosting_view_layout).unwrap().state(),
        NodeState::MaybeDirty
    );
    assert_eq!(
        graph.read_value(hosting_view_layout),
        Err(GraphError::MissingNode(avatar_size_preference))
    );
}

#[test]
fn unmounting_a_badge_view_invalidates_host_layout_and_never_serves_its_stale_cache() {
    let mut graph = AttributeGraph::new();
    let unread_badge = graph.create_subgraph(None).unwrap();
    let (text_intrinsic_width, badge_width) = graph
        .with_subgraph(unread_badge, |graph| {
            let text_intrinsic_width = graph.add_source(ValueStorage::from_i64(7));
            let badge_width = graph.add_derived(boxed_rule(
                ForwardViewMetricRule {
                    source_metric: text_intrinsic_width,
                },
                update_forwarded_view_metric,
                VIEW_METRIC,
                "UnreadBadge.width",
            ));
            (text_intrinsic_width, badge_width)
        })
        .unwrap();
    let toolbar_width = graph.add_derived(boxed_rule(
        ForwardViewMetricRule {
            source_metric: badge_width,
        },
        update_forwarded_view_metric,
        VIEW_METRIC,
        "Toolbar.width",
    ));
    let screen_width = graph.add_derived(boxed_rule(
        ForwardViewMetricRule {
            source_metric: toolbar_width,
        },
        update_forwarded_view_metric,
        VIEW_METRIC,
        "InboxScreen.width",
    ));

    assert_eq!(graph.read_value(screen_width).unwrap().as_i64(), Some(7));
    assert_eq!(graph.node(toolbar_width).unwrap().state(), NodeState::Clean);
    assert_eq!(graph.node(screen_width).unwrap().state(), NodeState::Clean);

    let removal = graph.remove_subgraph(unread_badge).unwrap();
    assert_eq!(removal.subgraphs, vec![unread_badge]);
    assert_eq!(removal.nodes, vec![text_intrinsic_width, badge_width]);
    assert_eq!(removal.dirtied_dependents, vec![toolbar_width]);
    assert!(!graph.contains_node(text_intrinsic_width));
    assert!(!graph.contains_node(badge_width));
    assert!(graph.contains_node(toolbar_width));
    assert!(graph.contains_node(screen_width));
    assert_eq!(graph.node(toolbar_width).unwrap().state(), NodeState::Dirty);
    assert_eq!(
        graph.node(screen_width).unwrap().state(),
        NodeState::MaybeDirty
    );
    assert_eq!(graph.dependencies_of(toolbar_width), Ok(vec![]));
    assert_eq!(graph.edges(), vec![Edge::new(toolbar_width, screen_width)]);
    assert_eq!(
        graph.pending_edges(),
        vec![Edge::new(toolbar_width, screen_width)]
    );
    assert_eq!(
        graph.read_value(screen_width),
        Err(GraphError::MissingNode(badge_width))
    );
    assert_eq!(
        graph.debug_cached_value(screen_width).unwrap().as_i64(),
        Some(7),
        "the old SwiftUI-style layout cache is diagnostic only and must not be served",
    );
    drop(removal);
}

#[test]
fn unmounting_an_inactive_if_branch_keeps_the_swiftui_else_content_usable() {
    let mut graph = AttributeGraph::new();
    let show_details = graph.add_source(ValueStorage::from_bool(true));
    let placeholder_height = graph.add_source(ValueStorage::from_i64(99));
    let detail_view = graph.create_subgraph(None).unwrap();
    let detail_height = add_view_metric(&mut graph, detail_view, 10);
    let visible_content_height = graph.add_derived(boxed_rule(
        ConditionalContentMetricRule {
            is_presented: show_details,
            presented_metric: detail_height,
            placeholder_metric: placeholder_height,
        },
        update_conditional_content_metric,
        VIEW_METRIC,
        "ConditionalContent.visibleHeight",
    ));

    assert_eq!(
        graph.read_value(visible_content_height).unwrap().as_i64(),
        Some(10)
    );
    graph
        .set_source_value(show_details, ValueStorage::from_bool(false))
        .unwrap();
    assert_eq!(
        graph.read_value(visible_content_height).unwrap().as_i64(),
        Some(99)
    );
    assert_eq!(
        graph.dependencies_of(visible_content_height),
        Ok(vec![show_details, placeholder_height])
    );

    let removal = graph.remove_subgraph(detail_view).unwrap();
    assert_eq!(
        graph.node(visible_content_height).unwrap().state(),
        NodeState::Clean
    );
    assert_eq!(
        graph.read_value(visible_content_height).unwrap().as_i64(),
        Some(99)
    );

    graph
        .set_source_value(show_details, ValueStorage::from_bool(true))
        .unwrap();
    assert_eq!(
        graph.read_value(visible_content_height),
        Err(GraphError::MissingNode(detail_height))
    );
    assert_eq!(
        graph.node(visible_content_height).unwrap().state(),
        NodeState::Dirty
    );

    graph
        .set_source_value(show_details, ValueStorage::from_bool(false))
        .unwrap();
    assert_eq!(
        graph.read_value(visible_content_height).unwrap().as_i64(),
        Some(99)
    );
    assert_eq!(
        graph.node(visible_content_height).unwrap().state(),
        NodeState::Clean
    );
    drop(removal);
}

#[test]
fn removing_one_swiftui_modifier_updates_view_membership_and_releases_each_rule_once() {
    let mut graph = AttributeGraph::new();
    let animated_card = graph.create_subgraph(None).unwrap();
    let destroy_count = Rc::new(Cell::new(0));
    let (opacity_modifier, offset_modifier) = graph
        .with_subgraph(animated_card, |graph| {
            let opacity_modifier = graph.add_derived(boxed_rule(
                ViewLifetimeRule {
                    destroy_count: Rc::clone(&destroy_count),
                    points: 1,
                },
                update_view_lifetime_metric,
                VIEW_METRIC,
                "AnimatedCard.opacityModifier",
            ));
            let offset_modifier = graph.add_derived(boxed_rule(
                ViewLifetimeRule {
                    destroy_count: Rc::clone(&destroy_count),
                    points: 2,
                },
                update_view_lifetime_metric,
                VIEW_METRIC,
                "AnimatedCard.offsetModifier",
            ));
            (opacity_modifier, offset_modifier)
        })
        .unwrap();

    let removed_opacity_modifier = graph.remove_node(opacity_modifier).unwrap();
    assert_eq!(removed_opacity_modifier.subgraph_id(), Some(animated_card));
    assert!(graph.contains_subgraph(animated_card));
    assert!(!graph.contains_node(opacity_modifier));
    assert!(graph.contains_node(offset_modifier));
    assert_eq!(destroy_count.get(), 0);

    let removed_card = graph.remove_subgraph(animated_card).unwrap();
    assert!(!graph.contains_node(offset_modifier));
    assert_eq!(removed_card.subgraphs, vec![animated_card]);
    assert_eq!(removed_card.nodes, vec![offset_modifier]);
    assert_eq!(removed_card.dirtied_dependents, vec![]);
    assert_eq!(destroy_count.get(), 1);
    drop(removed_card);
    assert_eq!(destroy_count.get(), 1);
    drop(removed_opacity_modifier);
    assert_eq!(destroy_count.get(), 2);
    drop(graph);
    assert_eq!(destroy_count.get(), 2);
}

#[test]
fn swiftui_view_scope_ids_are_not_reused_or_accepted_by_another_host_graph() {
    let mut previous_host_graph = AttributeGraph::new();
    let removed_row = previous_host_graph.create_subgraph(None).unwrap();
    drop(previous_host_graph.remove_subgraph(removed_row).unwrap());
    let replacement_row = previous_host_graph.create_subgraph(None).unwrap();
    assert_ne!(replacement_row, removed_row);

    let mut current_host_graph = AttributeGraph::new();
    let current_row = current_host_graph.create_subgraph(None).unwrap();
    let current_row_height = add_view_metric(&mut current_host_graph, current_row, 1);
    assert_ne!(current_row, removed_row);

    let callback_ran = Cell::new(false);
    assert_eq!(
        current_host_graph.with_subgraph(removed_row, |_graph| callback_ran.set(true)),
        Err(GraphError::GraphMismatch {
            expected: current_host_graph.id(),
            actual: removed_row.graph_id(),
        })
    );
    assert!(!callback_ran.get());
    assert_eq!(
        current_host_graph.create_subgraph(Some(removed_row)),
        Err(GraphError::GraphMismatch {
            expected: current_host_graph.id(),
            actual: removed_row.graph_id(),
        })
    );
    assert_eq!(
        current_host_graph.remove_subgraph(removed_row),
        Err(GraphError::GraphMismatch {
            expected: current_host_graph.id(),
            actual: removed_row.graph_id(),
        })
    );
    assert!(current_host_graph.subgraph(removed_row).is_none());
    assert!(!current_host_graph.contains_subgraph(removed_row));

    assert_eq!(previous_host_graph.subgraph_count(), 1);
    assert!(previous_host_graph.contains_subgraph(replacement_row));
    assert_eq!(current_host_graph.subgraph_count(), 1);
    assert!(current_host_graph.contains_subgraph(current_row));
    assert!(current_host_graph.contains_node(current_row_height));
    assert_eq!(current_host_graph.current_subgraph(), None);
}

#[test]
fn teardown_during_an_active_swiftui_builder_is_rejected_and_repeated_teardown_is_missing() {
    let mut graph = AttributeGraph::new();
    let settings_section = graph.create_subgraph(None).unwrap();
    let account_row = graph.create_subgraph(Some(settings_section)).unwrap();
    let section_spacing = add_view_metric(&mut graph, settings_section, 1);
    let account_row_height = add_view_metric(&mut graph, account_row, 2);

    graph
        .with_subgraph(account_row, |graph| {
            assert_eq!(
                graph.remove_subgraph(account_row),
                Err(GraphError::SubgraphInUse(account_row))
            );
            assert_eq!(
                graph.remove_subgraph(settings_section),
                Err(GraphError::SubgraphInUse(account_row))
            );
            assert_eq!(graph.current_subgraph(), Some(account_row));
            assert_eq!(graph.subgraph_count(), 2);
            assert!(graph.contains_node(section_spacing));
            assert!(graph.contains_node(account_row_height));
        })
        .unwrap();

    assert_eq!(graph.current_subgraph(), None);
    assert_eq!(graph.subgraph_count(), 2);
    let teardown_summary = graph.remove_subgraph(settings_section).unwrap();
    assert_eq!(
        graph.remove_subgraph(settings_section),
        Err(GraphError::MissingSubgraph(settings_section))
    );
    assert_eq!(
        graph.remove_subgraph(account_row),
        Err(GraphError::MissingSubgraph(account_row))
    );
    assert_eq!(graph.subgraph_count(), 0);
    assert_eq!(graph.node_count(), 0);
    drop(teardown_summary);
}

#[test]
fn swiftui_view_tree_teardown_releases_rules_once_after_structural_detach() {
    let destroy_count = Rc::new(Cell::new(0));
    let mut graph = AttributeGraph::new();
    let profile_card = graph.create_subgraph(None).unwrap();
    let name_label = graph.create_subgraph(Some(profile_card)).unwrap();

    let card_layout_rule = graph
        .with_subgraph(profile_card, |graph| {
            graph.add_derived(boxed_rule(
                ViewLifetimeRule {
                    destroy_count: Rc::clone(&destroy_count),
                    points: 1,
                },
                update_view_lifetime_metric,
                VIEW_METRIC,
                "ProfileCard.layout",
            ))
        })
        .unwrap();
    let label_layout_rule = graph
        .with_subgraph(name_label, |graph| {
            graph.add_derived(boxed_rule(
                ViewLifetimeRule {
                    destroy_count: Rc::clone(&destroy_count),
                    points: 2,
                },
                update_view_lifetime_metric,
                VIEW_METRIC,
                "NameLabel.layout",
            ))
        })
        .unwrap();

    let teardown_summary = graph.remove_subgraph(profile_card).unwrap();
    assert!(!graph.contains_subgraph(profile_card));
    assert!(!graph.contains_subgraph(name_label));
    assert!(!graph.contains_node(card_layout_rule));
    assert!(!graph.contains_node(label_layout_rule));
    assert_eq!(teardown_summary.subgraphs, vec![profile_card, name_label]);
    assert_eq!(
        teardown_summary.nodes,
        vec![card_layout_rule, label_layout_rule]
    );
    assert_eq!(teardown_summary.dirtied_dependents, vec![]);
    assert_eq!(
        destroy_count.get(),
        2,
        "every detached SwiftUI-style attribute rule is destroyed once"
    );

    drop(graph);
    assert_eq!(destroy_count.get(), 2);
    drop(teardown_summary);
    assert_eq!(destroy_count.get(), 2);
}

#[test]
fn dropping_a_host_graph_releases_rules_in_live_nested_swiftui_view_scopes_once() {
    let destroy_count = Rc::new(Cell::new(0));
    let mut host_graph = AttributeGraph::new();
    let settings_screen = host_graph.create_subgraph(None).unwrap();
    let account_row = host_graph.create_subgraph(Some(settings_screen)).unwrap();

    for (view_scope, points) in [(settings_screen, 1), (account_row, 2)] {
        host_graph
            .with_subgraph(view_scope, |graph| {
                graph.add_derived(boxed_rule(
                    ViewLifetimeRule {
                        destroy_count: Rc::clone(&destroy_count),
                        points,
                    },
                    update_view_lifetime_metric,
                    VIEW_METRIC,
                    "live SwiftUI-style view metric",
                ));
            })
            .unwrap();
    }

    assert_eq!(destroy_count.get(), 0);
    drop(host_graph);
    assert_eq!(destroy_count.get(), 2);
}
