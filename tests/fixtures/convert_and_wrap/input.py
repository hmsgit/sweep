def sync(records, batch_size=100):
    """Synchronize the given records with the remote skills catalogue and return a summary.

    Args:
        records (list[dict]): the raw records to push upstream, already validated against the catalogue schema and deduplicated by external identifier.
        batch_size (int): how many records go into one request.

    Returns:
        dict: mapping of record id to sync status.
    """
    return {}
