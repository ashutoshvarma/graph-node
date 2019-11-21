use futures::future::FutureResult;
use std::time::{SystemTime, UNIX_EPOCH};

use graph::data::subgraph::schema::*;
use graph::prelude::*;

pub fn check_subgraph_exists<S>(
    store: Arc<S>,
    subgraph_id: SubgraphDeploymentId,
) -> impl Future<Item = bool, Error = Error>
where
    S: Store,
{
    future::result(
        store
            .get(SubgraphDeploymentEntity::key(subgraph_id))
            .map_err(|e| e.into())
            .map(|entity| entity.map_or(false, |_| true)),
    )
}

pub fn create_subgraph<S>(
    store: Arc<S>,
    subgraph_name: SubgraphName,
    subgraph_id: SubgraphDeploymentId,
) -> FutureResult<(), Error>
where
    S: Store + ChainStore,
{
    let mut ops = vec![];

    // Ensure the subgraph itself doesn't already exist
    ops.push(MetadataOperation::AbortUnless {
        description: "Subgraph entity should not exist".to_owned(),
        query: SubgraphEntity::query()
            .filter(EntityFilter::new_equal("name", subgraph_name.to_string())),
        entity_ids: vec![],
    });

    // Create the subgraph entity (e.g. `ethereum/mainnet`)
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let subgraph_entity_id = generate_entity_id();
    ops.extend(
        SubgraphEntity::new(subgraph_name.clone(), None, None, created_at)
            .write_operations(&subgraph_entity_id)
            .into_iter()
            .map(|op| op.into()),
    );

    // Ensure the subgraph version doesn't already exist
    ops.push(MetadataOperation::AbortUnless {
        description: "Subgraph version should not exist".to_owned(),
        query: SubgraphVersionEntity::query()
            .filter(EntityFilter::new_equal("id", subgraph_id.to_string())),
        entity_ids: vec![],
    });

    // Create a subgraph version entity; we're using the same ID for
    // version and deployment to make clear they belong together
    let version_entity_id = subgraph_id.to_string();
    ops.extend(
        SubgraphVersionEntity::new(subgraph_entity_id.clone(), subgraph_id.clone(), created_at)
            .write_operations(&version_entity_id)
            .into_iter()
            .map(|op| op.into()),
    );

    // Immediately make this version the current one
    ops.extend(SubgraphEntity::update_pending_version_operations(
        &subgraph_entity_id,
        None,
    ));
    ops.extend(SubgraphEntity::update_current_version_operations(
        &subgraph_entity_id,
        Some(version_entity_id),
    ));

    // Ensure the deployment doesn't already exist
    ops.push(MetadataOperation::AbortUnless {
        description: "Subgraph deployment entity must not exist".to_owned(),
        query: SubgraphDeploymentEntity::query()
            .filter(EntityFilter::new_equal("id", subgraph_id.to_string())),
        entity_ids: vec![],
    });

    // Create a fake manifest
    let manifest = SubgraphManifest {
        id: subgraph_id.clone(),
        location: subgraph_name.to_string(),
        spec_version: String::from("0.0.1"),
        description: None,
        repository: None,
        schema: Schema::parse(include_str!("./ethereum.graphql"), subgraph_id.clone())
            .expect("valid Ethereum network subgraph schema"),
        data_sources: vec![],
        templates: vec![],
    };

    // Create deployment entity
    let chain_head_block = match store.chain_head_ptr() {
        Ok(block_ptr) => block_ptr,
        Err(e) => return future::err(e.into()),
    };
    ops.extend(
        SubgraphDeploymentEntity::new(&manifest, false, false, None, chain_head_block)
            .create_operations(&manifest.id),
    );

    // Create a deployment assignment entity
    ops.extend(
        SubgraphDeploymentAssignmentEntity::new(NodeId::new("__builtin").unwrap())
            .write_operations(&subgraph_id)
            .into_iter()
            .map(|op| op.into()),
    );

    future::result(
        store
            .create_subgraph_deployment(&manifest.schema, ops)
            .map_err(|e| e.into()),
    )
}
